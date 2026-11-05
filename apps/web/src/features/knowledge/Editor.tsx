import type { Editor as Tiptap } from '@tiptap/core'
import CodeBlockLowlight from '@tiptap/extension-code-block-lowlight'
import { common, createLowlight } from 'lowlight'
import Image from '@tiptap/extension-image'
import { TaskItem, TaskList } from '@tiptap/extension-list'
import Placeholder from '@tiptap/extension-placeholder'
import {
  Table,
  TableCell,
  TableHeader,
  TableRow,
} from '@tiptap/extension-table'
import { Markdown } from '@tiptap/markdown'
import { NodeSelection } from '@tiptap/pm/state'
import { EditorContent, ReactNodeViewRenderer, useEditor } from '@tiptap/react'
import { BubbleMenu } from '@tiptap/react/menus'
import StarterKit from '@tiptap/starter-kit'
import {
  Bold,
  Code as CodeIcon,
  GripVertical,
  ImagePlus,
  Italic,
  Link as LinkIcon,
  List,
  ListChecks,
  Plus,
  Strikethrough,
} from 'lucide-react'
import {
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type Ref,
} from 'react'
import { createPortal } from 'react-dom'
import { CodeView } from './CodeView'
import {
  blocks,
  SlashCommands,
  type Block,
  type BlockActions,
  type SlashView,
} from './slash'

export type EditorHandle = {
  load: (markdown: string) => void
  link: (name: string, href: string) => void
  image: (name: string, src: string) => void
}

function extensions(
  items: () => Block[],
  events: {
    show: (view: SlashView | null) => void
    key: (event: KeyboardEvent) => boolean
  },
  placeholder: string,
) {
  return [
    StarterKit.configure({
      codeBlock: false,
      link: {
        openOnClick: false,
        autolink: true,
        HTMLAttributes: { rel: 'noopener noreferrer', target: null },
      },
    }),
    CodeBlockLowlight.extend({
      addNodeView() {
        return ReactNodeViewRenderer(CodeView)
      },
    }).configure({ lowlight: createLowlight(common) }),
    Image.configure({ allowBase64: false }),
    TaskList,
    TaskItem.configure({ nested: true }),
    Table.configure({ resizable: false }),
    TableRow,
    // Markdown tables hold one line of text per cell, so a cell takes nothing more.
    TableHeader.extend({ content: 'paragraph' }),
    TableCell.extend({ content: 'paragraph' }),
    Markdown,
    Placeholder.configure({ placeholder }),
    SlashCommands(items, events),
  ]
}

type Callbacks = {
  onChange: ((markdown: string) => void) | undefined
  onFiles: ((files: File[]) => void) | undefined
  onOpenLink: (href: string) => void
  actions: BlockActions | undefined
}

/** What the editor, built once, calls: the latest props, and the open menu's keys. */
class Wires {
  private callbacks: Callbacks | null = null
  key: (event: KeyboardEvent) => boolean = () => false

  connect(callbacks: Callbacks) {
    this.callbacks = callbacks
  }
  blocks() {
    return blocks({
      pickImage: () => this.callbacks?.actions?.pickImage(),
      pickFile: () => this.callbacks?.actions?.pickFile(),
      pickPage: () => this.callbacks?.actions?.pickPage(),
    })
  }
  changed(markdown: string) {
    this.callbacks?.onChange?.(markdown)
  }
  clicked(event: MouseEvent) {
    const href = (event.target as Element).closest('a')?.getAttribute('href')
    if (!href) return false
    event.preventDefault()
    this.callbacks?.onOpenLink(href)
    return true
  }
  files(data: DataTransfer | null) {
    const files = [...(data?.files ?? [])]
    if (!files.length || !this.callbacks?.onFiles) return false
    this.callbacks.onFiles(files)
    return true
  }
  listen(keys: (event: KeyboardEvent) => boolean) {
    this.key = keys
  }
}

/** Loaded text is never written back unless a person changes it, so what an agent wrote stays as it wrote it. */
export function Editor({
  ref,
  markdown,
  editable = true,
  actions,
  onChange,
  onFiles,
  onOpenLink,
}: {
  ref?: Ref<EditorHandle>
  markdown: string
  editable?: boolean
  actions?: BlockActions
  onChange?: (markdown: string) => void
  onFiles?: (files: File[]) => void
  onOpenLink: (href: string) => void
}) {
  const [slash, setSlash] = useState<SlashView | null>(null)
  const [wires] = useState(() => new Wires())
  useLayoutEffect(() => {
    wires.connect({ onChange, onFiles, onOpenLink, actions })
  })
  const editor = useEditor({
    editable,
    immediatelyRender: true,
    shouldRerenderOnTransaction: false,
    extensions: extensions(
      () => wires.blocks(),
      { show: setSlash, key: (event) => wires.key(event) },
      'Write, or press / for blocks',
    ),
    content: markdown,
    contentType: 'markdown',
    onUpdate: ({ editor }) => wires.changed(editor.getMarkdown()),
    editorProps: {
      attributes: { class: 'kb-prose', 'aria-label': 'Page body' },
      handleClick: (_view, _pos, event) => wires.clicked(event),
      handleDrop: (_view, event) => wires.files(event.dataTransfer),
      handlePaste: (_view, event) => wires.files(event.clipboardData),
    },
  })
  useImperativeHandle(
    ref,
    () => ({
      load: (text) => {
        editor.commands.setContent(text, {
          contentType: 'markdown',
          emitUpdate: false,
        })
      },
      link: (name, href) => {
        editor
          .chain()
          .focus()
          .insertContent({
            type: 'text',
            text: name,
            marks: [{ type: 'link', attrs: { href } }],
          })
          .insertContent(' ')
          .run()
      },
      image: (name, src) => {
        editor.chain().focus().setImage({ src, alt: name }).run()
      },
    }),
    [editor],
  )
  return (
    <div className="kb-editor">
      {editable && <TouchBar editor={editor} actions={actions} />}
      {editable && <Formatting editor={editor} />}
      <EditorContent editor={editor} />
      {editable && <BlockHandle editor={editor} />}
      {slash && <SlashMenu key={slash.key} view={slash} wires={wires} />}
    </div>
  )
}

function SlashMenu({ view, wires }: { view: SlashView; wires: Wires }) {
  const [cursor, setCursor] = useState(0)
  const list = useRef<HTMLDivElement>(null)
  const items = view.items
  const at = Math.min(cursor, items.length - 1)
  useLayoutEffect(() => {
    wires.listen((event) => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        const step = event.key === 'ArrowDown' ? 1 : -1
        setCursor((at + step + items.length) % items.length)
        return true
      }
      if (event.key === 'Enter' && items[at]) {
        view.command(items[at])
        return true
      }
      return false
    })
    return () => wires.listen(() => false)
  })
  useLayoutEffect(() => {
    list.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  }, [at])
  const box = view.clientRect?.()
  if (!box || !items.length) return null
  const below = box.bottom + 300 < window.innerHeight
  return createPortal(
    <div
      ref={list}
      className="kb-slash"
      role="listbox"
      aria-label="Blocks"
      style={{
        left: Math.min(box.left, window.innerWidth - 300),
        ...(below
          ? { top: box.bottom + 6 }
          : { bottom: window.innerHeight - box.top + 6 }),
      }}
    >
      {items.map((block, index) => (
        <button
          key={block.name}
          type="button"
          role="option"
          aria-selected={index === at}
          className="kb-slash__item"
          onMouseEnter={() => setCursor(index)}
          // Keeps the editor's focus and selection through the click.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => view.command(block)}
        >
          <span className="kb-slash__icon">
            <block.icon />
          </span>
          <span className="kb-slash__text">
            <strong>{block.name}</strong>
            <span className="small faint">{block.hint}</span>
          </span>
        </button>
      ))}
    </div>,
    document.body,
  )
}

function Mark({
  label,
  active,
  onPress,
  children,
}: {
  label: string
  active?: boolean | undefined
  onPress: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      className="btn btn--ghost btn--icon btn--sm"
      aria-label={label}
      aria-pressed={active}
      title={label}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onPress}
    >
      {children}
    </button>
  )
}

/** Asks for a link's address in place of the browser's prompt, which standalone PWAs may lack. */
function LinkField({ editor, onDone }: { editor: Tiptap; onDone: () => void }) {
  const [href, setHref] = useState(
    () => (editor.getAttributes('link').href as string | undefined) ?? '',
  )
  return (
    <form
      className="kb-format__link"
      onSubmit={(event) => {
        event.preventDefault()
        const chain = editor.chain().focus().extendMarkRange('link')
        if (href.trim()) chain.setLink({ href: href.trim() }).run()
        else chain.unsetLink().run()
        onDone()
      }}
    >
      <input
        autoFocus
        className="input"
        aria-label="Link address"
        placeholder="https://"
        value={href}
        onChange={(event) => setHref(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Escape') onDone()
        }}
      />
    </form>
  )
}

function Formatting({ editor }: { editor: Tiptap }) {
  const [linking, setLinking] = useState(false)
  return (
    <BubbleMenu
      editor={editor}
      className="kb-format"
      shouldShow={({ editor, state }) =>
        !state.selection.empty &&
        !(state.selection instanceof NodeSelection) &&
        !editor.isActive('codeBlock')
      }
    >
      {linking ? (
        <LinkField editor={editor} onDone={() => setLinking(false)} />
      ) : (
        <>
          <Mark
            label="Bold"
            active={editor.isActive('bold')}
            onPress={() => editor.chain().focus().toggleBold().run()}
          >
            <Bold />
          </Mark>
          <Mark
            label="Italic"
            active={editor.isActive('italic')}
            onPress={() => editor.chain().focus().toggleItalic().run()}
          >
            <Italic />
          </Mark>
          <Mark
            label="Strikethrough"
            active={editor.isActive('strike')}
            onPress={() => editor.chain().focus().toggleStrike().run()}
          >
            <Strikethrough />
          </Mark>
          <Mark
            label="Code"
            active={editor.isActive('code')}
            onPress={() => editor.chain().focus().toggleCode().run()}
          >
            <CodeIcon />
          </Mark>
          <Mark
            label="Link"
            active={editor.isActive('link')}
            onPress={() => setLinking(true)}
          >
            <LinkIcon />
          </Mark>
        </>
      )}
    </BubbleMenu>
  )
}

/** Phones have no pointer to hover a block or select precisely, so these stay within reach. */
function TouchBar({
  editor,
  actions,
}: {
  editor: Tiptap
  actions: BlockActions | undefined
}) {
  return (
    <div className="kb-touchbar" role="toolbar" aria-label="Formatting">
      <Mark
        label="Add a block"
        onPress={() => editor.chain().focus().insertContent('/').run()}
      >
        <Plus />
      </Mark>
      <Mark
        label="Bold"
        onPress={() => editor.chain().focus().toggleBold().run()}
      >
        <Bold />
      </Mark>
      <Mark
        label="Italic"
        onPress={() => editor.chain().focus().toggleItalic().run()}
      >
        <Italic />
      </Mark>
      <Mark
        label="Bulleted list"
        onPress={() => editor.chain().focus().toggleBulletList().run()}
      >
        <List />
      </Mark>
      <Mark
        label="To-do list"
        onPress={() => editor.chain().focus().toggleTaskList().run()}
      >
        <ListChecks />
      </Mark>
      <Mark label="Image" onPress={() => actions?.pickImage()}>
        <ImagePlus />
      </Mark>
    </div>
  )
}

/** ProseMirror moves whatever `view.dragging` holds when a drag lands in the document. */
function beginMove(view: Tiptap['view'], selection: NodeSelection) {
  view.dragging = { slice: selection.content(), move: true }
}

function BlockHandle({ editor }: { editor: Tiptap }) {
  const [block, setBlock] = useState<{ pos: number; top: number } | null>(null)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const scope = root.current?.closest('.kb-editor')
    const move = (event: MouseEvent) => {
      // The view exists only once the editor has mounted, a moment after this first runs.
      if (!root.current || !editor.isInitialized) return
      const view = editor.view
      const box = view.dom.getBoundingClientRect()
      // A wide margin on the left, so moving toward the handle does not lose the block.
      const hit = view.posAtCoords({
        left: Math.max(event.clientX, box.left + 2),
        top: event.clientY,
      })
      if (!hit || hit.inside < 0) return
      const resolved = view.state.doc.resolve(hit.inside)
      const pos = resolved.depth ? resolved.before(1) : hit.inside
      const element = view.nodeDOM(pos)
      if (!(element instanceof HTMLElement)) return
      const parent = root.current.parentElement?.getBoundingClientRect()
      const style = getComputedStyle(element)
      setBlock({
        pos,
        top:
          element.getBoundingClientRect().top -
          (parent?.top ?? 0) +
          Math.min(parseFloat(style.lineHeight) / 2 - 12 || 0, 12),
      })
    }
    scope?.addEventListener('mousemove', move as EventListener)
    return () => scope?.removeEventListener('mousemove', move as EventListener)
  }, [editor])
  const select = () => {
    if (!block) return null
    const selection = NodeSelection.create(editor.state.doc, block.pos)
    editor.view.dispatch(editor.state.tr.setSelection(selection))
    return selection
  }
  return (
    <div
      ref={root}
      className="kb-handle"
      hidden={!block}
      style={{ top: block?.top ?? 0 }}
      contentEditable={false}
    >
      <button
        type="button"
        className="kb-handle__button"
        aria-label="Add a block below"
        title="Add a block below"
        onClick={() => {
          if (!block) return
          const node = editor.state.doc.nodeAt(block.pos)
          if (!node) return
          const after = block.pos + node.nodeSize
          editor
            .chain()
            .focus()
            .insertContentAt(after, { type: 'paragraph' })
            .setTextSelection(after + 1)
            .insertContent('/')
            .run()
        }}
      >
        <Plus />
      </button>
      <span
        className="kb-handle__button kb-handle__grip"
        draggable
        title="Drag to move"
        aria-hidden="true"
        onDragStart={(event) => {
          const selection = select()
          if (!selection) return
          const element = editor.view.nodeDOM(selection.from)
          if (element instanceof HTMLElement)
            event.dataTransfer.setDragImage(element, 0, 0)
          event.dataTransfer.effectAllowed = 'move'
          beginMove(editor.view, selection)
          event.dataTransfer.setData('text/plain', selection.node.textContent)
        }}
      >
        <GripVertical />
      </span>
    </div>
  )
}
