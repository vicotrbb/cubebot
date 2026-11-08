import { createFileRoute } from '@tanstack/react-router'
import { Sidebar } from '../features/shell/Sidebar'

/** On a phone, Home is the landing page; agents, teams and rooms are one tap away here. */
export const Route = createFileRoute('/_app/menu')({
  component: () => <Sidebar page />,
})
