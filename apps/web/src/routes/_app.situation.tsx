import { createFileRoute, redirect } from '@tanstack/react-router'

/** The Situation room became Home; old links and installed shortcuts land there. */
export const Route = createFileRoute('/_app/situation')({
  beforeLoad: () => {
    throw redirect({ to: '/', replace: true })
  },
})
