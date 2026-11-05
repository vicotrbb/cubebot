import { createContext, useContext } from 'react'
import type { Turn } from '../../api/client'
import type { AgentSummary, Session } from '../../api/types'

type AgentScreen = {
  agent: AgentSummary
  session: Session
  turns: Turn[]
  streaming: Record<string, string>
  addTurn: (turn: Turn) => void
}

const Context = createContext<AgentScreen | null>(null)
export const AgentProvider = Context.Provider

export function useAgentScreen() {
  const value = useContext(Context)
  if (!value) throw new Error('useAgentScreen outside an agent route')
  return value
}
