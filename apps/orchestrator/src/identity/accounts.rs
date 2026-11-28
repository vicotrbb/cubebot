use uuid::Uuid;

use super::{Actor, Identity, IdentityError, Receipt, Role, commands::Command, members, passwords};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct AccountSummary {
    pub id: Uuid,
    pub login: String,
    pub name: Option<String>,
    pub administrator: bool,
    pub role: Role,
    /// The installation owner.
    pub owner: bool,
    pub active: bool,
    pub revision: i64,
    pub two_factor: bool,
    pub last_seen_at_unix: Option<i64>,
    /// Live agents, rooms and teams this person owns.
    pub owned: i64,
    pub sessions: i64,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn account_login(&self, actor: &Actor) -> Result<String, IdentityError> {
        sqlx::query_scalar!(
            "
            SELECT login
            FROM human_accounts
            WHERE id = $1 AND active
            ",
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(IdentityError::Unauthorized)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn accounts(
        &self,
        actor: &Actor,
        after: Option<Uuid>,
    ) -> Result<Vec<AccountSummary>, IdentityError> {
        self.require_overseer(actor).await?;
        let rows = sqlx::query!(
            r#"
            SELECT
                a.id AS "id!",
                a.login AS "login!",
                NULLIF(a.profile->>'name', '') AS name,
                a.role AS "role!",
                i.owner_id IS NOT DISTINCT FROM a.id AS "owner!",
                a.active AS "active!",
                a.revision AS "revision!",
                a.totp_enabled_at IS NOT NULL AS "two_factor!",
                EXTRACT(EPOCH FROM a.last_seen_at)::bigint AS last_seen_at_unix,
                (SELECT count(*) FROM agents g WHERE g.created_by = a.id AND g.desired_state <> 'retired')
                    + (SELECT count(*) FROM rooms r WHERE r.created_by = a.id AND NOT r.task AND NOT r.archived)
                    + (SELECT count(*) FROM teams t WHERE t.created_by = a.id AND t.deleted_at IS NULL) AS "owned!",
                (SELECT count(*) FROM human_sessions s WHERE s.account_id = a.id AND s.expires_at > clock_timestamp()) AS "sessions!"
            FROM human_accounts a CROSS JOIN installation i
            WHERE $1::uuid IS NULL OR a.id > $1
            ORDER BY a.id
            LIMIT 100
            "#,
            after,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| AccountSummary {
                id: row.id,
                login: row.login,
                name: row.name,
                administrator: row.role == "administrator",
                role: Role::parse(&row.role),
                owner: row.owner,
                active: row.active,
                revision: row.revision,
                two_factor: row.two_factor,
                last_seen_at_unix: row.last_seen_at_unix,
                owned: row.owned,
                sessions: row.sessions,
            })
            .collect())
    }

    pub async fn setup_open(&self) -> Result<bool, IdentityError> {
        let bootstrapped = sqlx::query_scalar!(
            r#"
            SELECT bootstrapped AS "bootstrapped!"
            FROM installation
            WHERE singleton
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(!bootstrapped)
    }

    #[tracing::instrument(skip_all)]
    pub async fn bootstrap(&self, login: &str, password: &str) -> Result<Uuid, IdentityError> {
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let mut transaction = self.pool.begin().await?;
        let bootstrapped = sqlx::query_scalar!(
            r#"
            SELECT bootstrapped AS "bootstrapped!"
            FROM installation
            WHERE singleton
            FOR UPDATE
            "#,
        )
        .fetch_one(&mut *transaction)
        .await?;
        if bootstrapped {
            return Err(IdentityError::Conflict);
        }
        let id = insert_account(&mut transaction, login, &verifier, "administrator").await?;
        sqlx::query!(
            "
            UPDATE installation
            SET bootstrapped = true, owner_id = $1
            WHERE singleton
            ",
            id,
        )
        .execute(&mut *transaction)
        .await?;
        audit(&mut transaction, id, id, "account.bootstrap").await?;
        transaction.commit().await?;
        Ok(id)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %request_id))]
    pub async fn create_account(
        &self,
        actor: &Actor,
        login: &str,
        password: &str,
        administrator: bool,
        request_id: Uuid,
    ) -> Result<Uuid, IdentityError> {
        let role = if administrator {
            Role::Administrator
        } else {
            Role::Member
        };
        self.create_account_as(actor, login, password, role, request_id)
            .await
    }

    /// Only the installation owner creates administrators.
    #[tracing::instrument(skip_all, fields(actor = %actor.id, request = %request_id))]
    pub async fn create_account_as(
        &self,
        actor: &Actor,
        login: &str,
        password: &str,
        role: Role,
        request_id: Uuid,
    ) -> Result<Uuid, IdentityError> {
        let administrator = role == Role::Administrator;
        let command = Command::new(
            actor,
            "account.create",
            request_id,
            &account_input(login, role),
        )?;
        let (transaction, previous) = command.begin(&self.pool).await?;
        transaction.rollback().await?;
        if let Some(receipt) = previous {
            return self
                .replay_account(actor, request_id, password, receipt)
                .await;
        }
        passwords::validate(login, password)?;
        let verifier = passwords::hash(password).await?;
        let command = command.with_password_verifier(verifier.clone());
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            transaction.rollback().await?;
            return self
                .replay_account(actor, request_id, password, receipt)
                .await;
        }
        if administrator && !members::is_owner(&mut transaction, actor.id).await? {
            return Err(IdentityError::Forbidden);
        }
        let id = insert_account(&mut transaction, login, &verifier, role.as_str()).await?;
        command
            .commit(transaction, Receipt { id, revision: 1 })
            .await?;
        Ok(id)
    }

    async fn replay_account(
        &self,
        actor: &Actor,
        request_id: Uuid,
        password: &str,
        receipt: Receipt,
    ) -> Result<Uuid, IdentityError> {
        self.verify_command_password(actor.id, "account.create", request_id, password)
            .await?;
        self.require_administrator(actor).await?;
        Ok(receipt.id)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, subject = %subject, request = %request_id))]
    pub async fn suspend(
        &self,
        actor: &Actor,
        subject: Uuid,
        expected_revision: i64,
        request_id: Uuid,
    ) -> Result<Receipt, IdentityError> {
        let input = serde_json::json!({"id": subject, "expected_revision": expected_revision});
        let command = Command::new(actor, "account.suspend", request_id, &input)?;
        let (mut transaction, previous) = command.begin(&self.pool).await?;
        if let Some(receipt) = previous {
            return Ok(receipt);
        }
        let found = members::subject(&mut transaction, subject).await?;
        members::guard_subject(&mut transaction, actor, &found, false).await?;
        let revision = sqlx::query_scalar!(
            r#"
            UPDATE human_accounts
            SET active = false, revision = revision + 1
            WHERE id = $1 AND revision = $2 AND active
            RETURNING revision AS "revision!"
            "#,
            subject,
            expected_revision,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = revision.ok_or(IdentityError::Conflict)?;
        sqlx::query!(
            "
            DELETE FROM human_sessions
            WHERE account_id = $1
            ",
            subject,
        )
        .execute(&mut *transaction)
        .await?;
        command
            .commit(
                transaction,
                Receipt {
                    id: subject,
                    revision,
                },
            )
            .await
    }
}

pub(super) async fn lock_administrator(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
) -> Result<(), IdentityError> {
    let allowed = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!"
        FROM human_accounts
        WHERE id = $1 AND active AND role = 'administrator'
        FOR UPDATE
        "#,
        actor,
    )
    .fetch_optional(&mut **transaction)
    .await?;
    allowed.ok_or(IdentityError::Forbidden)?;
    Ok(())
}

/// The audited input; members and administrators keep the shape older receipts were made with.
fn account_input(login: &str, role: Role) -> serde_json::Value {
    match role {
        Role::Administrator | Role::Member => {
            serde_json::json!({"login": login, "administrator": role == Role::Administrator})
        }
        _ => serde_json::json!({"login": login, "role": role}),
    }
}

pub(super) async fn insert_account(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    login: &str,
    verifier: &str,
    role: &str,
) -> Result<Uuid, IdentityError> {
    Ok(sqlx::query_scalar!(
        r#"
        INSERT INTO human_accounts (login, password_hash, role)
        VALUES ($1, $2, $3)
        RETURNING id AS "id!"
        "#,
        login,
        verifier,
        role,
    )
    .fetch_one(&mut **transaction)
    .await?)
}

pub(super) async fn audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Uuid,
    subject: Uuid,
    kind: &'static str,
) -> Result<(), sqlx::Error> {
    let detail = match kind {
        "session.sign_in" => serde_json::json!({"method": "oidc"}),
        _ => serde_json::json!({}),
    };
    let event = super::audit::Event::by(actor, kind)
        .on(subject)
        .detail(detail);
    super::audit::record(&mut **transaction, &event).await
}
