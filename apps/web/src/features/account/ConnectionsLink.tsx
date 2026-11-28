import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useSession } from '../../app/queries'

/** Where someone connects their own accounts; administrators connect for everyone instead. */
export function ConnectionsLink() {
  const session = useSession()
  const administrator = session.data?.administrator === true
  return (
    <section className="profile__section" aria-labelledby="your-connections">
      <div className="profile__text">
        <h3 id="your-connections">
          {administrator ? 'Installation connections' : 'Your connections'}
        </h3>
        <p className="faint small">
          {administrator
            ? 'As an administrator, the accounts you connect serve the whole installation.'
            : 'Your own accounts, such as GitHub or Google, for the agents you manage.'}
        </p>
      </div>
      <Link
        to={administrator ? '/platform/connections' : '/connections'}
        className="btn btn--sm"
      >
        Open <ArrowRight />
      </Link>
    </section>
  )
}
