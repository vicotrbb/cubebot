import {
  BookText,
  Building2,
  CalendarClock,
  FolderGit2,
  Inbox,
  Mail,
  MessageCircle,
  Plug,
  Send,
  Webhook,
  type LucideIcon,
} from 'lucide-react'

/** No vendor logos: each connector gets a glyph for what it is. */
const GLYPHS: Record<string, LucideIcon> = {
  email: Mail,
  google: Inbox,
  microsoft: Building2,
  dav: CalendarClock,
  github: FolderGit2,
  notion: BookText,
  slack: MessageCircle,
  telegram: Send,
  'http-api': Webhook,
}

export function Mark({ id, size = 'md' }: { id: string; size?: 'md' | 'lg' }) {
  const Glyph = GLYPHS[id] ?? Plug
  return (
    <span className={`mark mark--${size}`} aria-hidden="true">
      <Glyph />
    </span>
  )
}
