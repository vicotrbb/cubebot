# ADR 0056: Shared installation skill library by default

Status: Accepted

Date: 2026-10-23

Skills created during work and dreaming should help other agents reuse successful procedures. Cubebot needs a default visibility model that distinguishes access to reusable packages from changes to an agent's installed behavior.

Decision: Newly registered skills enter a shared installation library by default and are discoverable by other agents. Each agent independently selects which available skills and revisions to install. The user selected this model on 2026-10-23. Sharing within the installation does not publish a skill externally or automatically install it everywhere.

Usage: One agent develops a diagnostic skill and registers it during a dream. Another agent searches the library, reads the relevant package, and installs its selected revision for future work. The first agent later publishes an improvement; the second agent's installation remains unchanged until it selects an update.

The default applies to skills created by agents during ordinary work or dreaming, and to skills registered by authorized humans through creation or import. Library discovery includes access to the package needed to evaluate and use it. Per-agent installation records remain separate from shared library records under ADR 0055. Discovery, installation, and execution do not grant additional tools, credentials, or resource access.

Ownership: The skills module owns shared catalog entries, immutable revisions, and per-agent installation records. [ADR 0057](0057-collaborative-shared-skill-revisions.md) permits any active agent or admitted active user to publish revisions. Authorization retains responsibility for separate operations such as removal or management of another agent's installations.

Reusable packages must be written with their shared audience in mind. An agent's private task history does not become generally readable merely because it produced a skill. Generated instructions should extract procedures without embedding credentials, private transcripts, or unnecessary source-specific data. Internal provenance can link to protected source records, whose original access checks still apply. Discovering a package cannot authorize following a protected source link. Automated checks cannot guarantee that generated content contains no sensitive information, so sharing and provenance must remain inspectable.

Human members can read shared packages and contribute revisions under ADR 0057. These capabilities do not grant unrestricted library administration or access to protected underlying histories. Administrators retain their existing content-access rights.

Skill registration and dream results must make shared-library publication visible, including identity and revision. Publication does not prove that other agents adopted the skill. A failed registration must not appear discoverable, and a successful registration followed by failed local installation must remain a distinct recoverable outcome.

Alternative considered: Keep skills private to the creating agent until explicitly shared. The selected shared default supports installation-wide reuse and follows OpenBot's shared local-library direction while retaining independent installations.

Open details: Removal and other library-administration authority; any future restricted visibility option; catalog ranking; provenance presentation; and package withdrawal behavior.

Validation scenarios: Discover a newly created or imported skill from another agent; install a selected revision; keep other installations unchanged after publication; deny tool or source access not otherwise granted; distinguish shared publication from adoption; and keep contribution rights separate from removal and installation-management rights.
