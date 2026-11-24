import { useQuery } from '@tanstack/react-query'
import { api, ok } from '../../api/client'
import type { Schemas } from '../../api/types'

export type Finding = Schemas['Finding']
export type Mute = Schemas['Mute']
export type Check = Schemas['Check']
export type Severity = 'high' | 'medium' | 'low'

export const WATCHDOG_KEY = ['watchdog'] as const

export function useWatchdog() {
  return useQuery({
    queryKey: WATCHDOG_KEY,
    queryFn: () => ok(api.GET('/api/watchdog')),
    refetchInterval: 15_000,
  })
}

const RULES: Record<string, string> = {
  reverse_shell: 'Reverse shell',
  miner: 'Cryptocurrency miner',
  credential_reach: 'Reached for credentials',
  privilege_probe: 'Privilege probe',
  secret_sent: 'Credential sent out',
  injection_then_send: 'Possible prompt injection',
  shared_content: 'Dangerous shared content',
  send_burst: 'Burst of sends',
  vault_burst: 'Vault burst',
  sign_in_failures: 'Failed sign-ins',
  spend_spike: 'Spending spike',
  tool_loop: 'Repeated tool call',
  sensitive_change: 'Sensitive change',
  privilege_escalation: 'Privilege escalation',
  miner_process: 'Miner process',
  hidden_executable: 'Hidden executable',
  process_flood: 'Process flood',
  blocked_private: 'Blocked private address',
  lan_scan: 'Network scan',
  odd_port: 'Suspicious port',
  tampered: 'System file changed',
  setuid_file: 'Setuid file planted',
  sensor_silent: 'Sensor went silent',
  large_upload: 'Large upload',
  idle_traffic: 'Traffic with no work',
  dns_tunnel: 'DNS tunnelling',
  direct_ip: 'Connection without DNS',
  foreign_pod: 'Unknown pod',
  image_swapped: 'Image swapped',
  oom_killed: 'Out of memory',
  crash_loop: 'Crash loop',
  administrator_added: 'Administrator added',
  access_granted: 'Access granted',
  provider_changed: 'Provider changed',
  refusal_burst: 'Refused requests',
}

export const ruleTitle = (rule: string) =>
  RULES[rule] ??
  rule.charAt(0).toUpperCase() + rule.slice(1).replaceAll('_', ' ')

export const SEVERITY: Record<Severity, string> = {
  high: 'High',
  medium: 'Medium',
  low: 'Low',
}

export const severityOf = (finding: Pick<Finding, 'severity'>): Severity =>
  finding.severity === 'high' || finding.severity === 'medium'
    ? finding.severity
    : 'low'

/** The page head's few words: how many are open and how many of those are high. */
export function headline(findings: Pick<Finding, 'severity'>[]) {
  if (findings.length === 0) return 'nothing open'
  const high = findings.filter((finding) => finding.severity === 'high').length
  return high > 0 && high < findings.length
    ? `${findings.length} open · ${high} high`
    : `${findings.length} open`
}

/** Who a mute or a mute-and-dismiss covers, in the words of its menu item. */
export const scopeOf = (agentName: string | null | undefined) =>
  agentName ? `for ${agentName}` : 'everywhere'

/** Findings that notify: the sidebar badge counts these, as Activity lists them. */
export const notifying = (items: { kind: string }[] | undefined) =>
  items?.filter((item) => item.kind === 'watchdog').length ?? 0
