import { ArrowUpRight } from 'lucide-react'
import { Link } from '@tanstack/react-router'
import type { components } from '../../api/schema'
import { useInstallation } from '../../app/queries'
import { CopyButton, Skeleton } from '../../components/primitives'
import { fileSize } from '../files/viewer'
import { useInstallationAbout } from '../shell/UpdateHint'
import { DEPLOYMENTS, plural, uptime } from './facts'
import { PROVIDER_NAMES } from './models'
import { sandboxLabel } from './sandbox'
import { Section } from './Section'

type Facts = components['schemas']['About']

export function About() {
  const about = useInstallationAbout()
  return (
    <Section
      title="About"
      description="What is running here, read from the orchestrator and its database when this page opened."
    >
      {about.data ? (
        <Facts facts={about.data} />
      ) : about.isError ? (
        <p className="small faint">
          The orchestrator did not answer. Retry shortly.
        </p>
      ) : (
        <Skeleton lines={4} />
      )}
    </Section>
  )
}

function Facts({ facts }: { facts: Facts }) {
  const installation = useInstallation().data
  const [deployment, how] = DEPLOYMENTS[installation?.controller ?? ''] ?? [
    installation?.controller ?? '…',
    '',
  ]
  const installed = new Date(facts.installed_at).toLocaleDateString(undefined, {
    dateStyle: 'long',
  })
  return (
    <dl className="about">
      <div>
        <dt>Version</dt>
        <Version facts={facts} />
      </div>
      <div>
        <dt>Deployment</dt>
        <dd>
          {deployment}
          <span className="faint">
            {facts.namespace ? `Namespace ${facts.namespace}` : how}
          </span>
        </dd>
      </div>
      <div>
        <dt>Agent sandbox</dt>
        <dd>
          {installation ? sandboxLabel(installation) : '…'}
          <span className="faint">
            Terminal {installation?.terminal_enabled ? 'on' : 'off'}, computer
            use {installation?.computer_use_enabled ? 'on' : 'off'}
          </span>
        </dd>
      </div>
      <div>
        <dt>Default model</dt>
        <dd>
          {facts.provider && facts.model ? (
            <>
              <code>{facts.model}</code>
              <span className="faint">
                {PROVIDER_NAMES[
                  facts.provider as keyof typeof PROVIDER_NAMES
                ] ?? facts.provider}
              </span>
            </>
          ) : (
            <Link to="/platform/providers">Set one up under AI providers</Link>
          )}
        </dd>
      </div>
      <div>
        <dt>Running for</dt>
        <dd>
          {uptime(facts.uptime_seconds)}
          <span className="faint">Installed {installed}</span>
        </dd>
      </div>
      <div>
        <dt>Database</dt>
        <dd>
          PostgreSQL {facts.postgres_version.split(' ')[0]}
          <span className="faint tabular">
            {fileSize(facts.database_bytes)} stored
          </span>
        </dd>
      </div>
      <div>
        <dt>On this installation</dt>
        <dd>
          {plural(facts.people, 'person', 'people')},{' '}
          {plural(facts.agents, 'agent')}
          <span className="faint">
            {plural(facts.rooms, 'room')}, {plural(facts.teams, 'team')}
          </span>
        </dd>
      </div>
      <div>
        <dt>Installation ID</dt>
        <dd className="about__id">
          <code title={facts.installation_id}>{facts.installation_id}</code>
          <CopyButton text={facts.installation_id} label="Copy the ID" />
        </dd>
      </div>
    </dl>
  )
}

function Version({ facts }: { facts: Facts }) {
  return (
    <dd className="about__version">
      <strong className="tabular">{facts.version}</strong>
      {facts.newer_release && (
        <a
          className="about__newer"
          href={facts.newer_release.url}
          target="_blank"
          rel="noreferrer"
        >
          {facts.newer_release.version} is available
          <ArrowUpRight aria-hidden="true" />
        </a>
      )}
    </dd>
  )
}
