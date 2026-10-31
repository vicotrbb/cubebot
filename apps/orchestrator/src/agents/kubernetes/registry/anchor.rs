use super::{Error, Registry};
use k8s_openapi::{api::core::v1::ConfigMap, apimachinery::pkg::apis::meta::v1::ObjectMeta};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(super) struct Binding {
    pub installation: Uuid,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}
struct Anchor {
    installation: Uuid,
    namespace: String,
    anchor_name: String,
    anchor_attempt: Uuid,
    anchor_uid: Option<String>,
}

impl Registry {
    #[tracing::instrument(skip_all)]
    pub async fn configure(&self, namespace: &str) -> Result<ConfigMap, Error> {
        super::super::client::validate_namespace(namespace)?;
        let installation = sqlx::query_scalar!(
            "
            SELECT id
            FROM installation
            WHERE singleton
            "
        )
        .fetch_one(&self.pool)
        .await?;
        sqlx::query!(
            "
            INSERT INTO installation_kubernetes (singleton, namespace, anchor_name)
            VALUES (true, $1, $2)
            ON CONFLICT (singleton) DO NOTHING
            ",
            namespace,
            format!("cb-install-{}", installation.simple()),
        )
        .execute(&self.pool)
        .await?;
        let row = sqlx::query_as!(
            Anchor,
            "
            SELECT i.id AS installation, k.namespace, k.anchor_name, k.anchor_attempt, k.anchor_uid
            FROM installation_kubernetes k
            CROSS JOIN installation i
            WHERE k.singleton
              AND i.singleton
            "
        )
        .fetch_one(&self.pool)
        .await?;
        if row.namespace != namespace {
            return Err(Error::Conflict);
        }
        Ok(manifest(&row))
    }
    #[tracing::instrument(skip_all)]
    pub async fn bind_anchor(&self, observed: &ConfigMap) -> Result<(), Error> {
        let mut transaction = self.transaction().await?;
        let row = sqlx::query_as!(
            Anchor,
            "
            SELECT i.id AS installation, k.namespace, k.anchor_name, k.anchor_attempt, k.anchor_uid
            FROM installation_kubernetes k
            CROSS JOIN installation i
            WHERE k.singleton
              AND i.singleton
            FOR UPDATE OF k
            "
        )
        .fetch_one(&mut *transaction)
        .await?;
        let expected = manifest(&row);
        super::objects::verify_metadata(&expected.metadata, &observed.metadata)?;
        if expected.data != observed.data || observed.immutable != Some(true) {
            return Err(Error::OwnershipMismatch);
        }
        let uid = super::objects::uid(&observed.metadata)?;
        let version = observed
            .metadata
            .resource_version
            .as_deref()
            .ok_or(Error::OwnershipMismatch)?;
        sqlx::query!(
            "
            UPDATE installation_kubernetes
            SET anchor_uid = $1,
                anchor_resource_version = $2
            WHERE singleton
            ",
            uid,
            version,
        )
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(super) async fn load(transaction: &mut Transaction<'_, Postgres>) -> Result<Binding, Error> {
    let row = sqlx::query_as!(
        Anchor,
        "
        SELECT i.id AS installation, k.namespace, k.anchor_name, k.anchor_attempt, k.anchor_uid
        FROM installation_kubernetes k
        CROSS JOIN installation i
        WHERE k.singleton
          AND i.singleton
        "
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(Error::Conflict)?;
    Ok(Binding {
        installation: row.installation,
        namespace: row.namespace,
        name: row.anchor_name,
        uid: row.anchor_uid.ok_or(Error::Conflict)?,
    })
}
fn manifest(row: &Anchor) -> ConfigMap {
    ConfigMap {
        metadata: ObjectMeta {
            name: Some(row.anchor_name.clone()),
            namespace: Some(row.namespace.clone()),
            uid: row.anchor_uid.clone(),
            labels: Some(BTreeMap::from([
                (
                    "cubebot.dev/installation".into(),
                    row.installation.to_string(),
                ),
                (
                    "cubebot.dev/anchor-attempt".into(),
                    row.anchor_attempt.to_string(),
                ),
            ])),
            finalizers: Some(vec!["cubebot.dev/installation-retained".into()]),
            ..Default::default()
        },
        immutable: Some(true),
        data: Some(BTreeMap::from([(
            "installation".into(),
            row.installation.to_string(),
        )])),
        ..Default::default()
    }
}
