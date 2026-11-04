import { Extension, type Editor, type Range } from '@tiptap/core'
import { PluginKey } from '@tiptap/pm/state'
import Suggestion, { type SuggestionProps } from '@tiptap/suggestion'
import {
  Code,
  FileUp,
  Heading1,
  Heading2,
  Heading3,
  Image as ImageIcon,
  Link2,
  List,
  ListChecks,
  ListOrdered,
  Minus,
  Pilcrow,
  Quote,
  Table,
  Workflow,
  type LucideIcon,
} from 'lucide-react'

export type Block = {
  name: string
  hint: string
  words: string
  icon: LucideIcon
  run: (editor: Editor) => void
}

/** What only the page can do: open a file picker or a page picker. */
export type BlockActions = {
  pickImage: () => void
  pickFile: () => void
  pickPage: () => void
}

const DIAGRAM = 'flowchart LR\n  A[Start] --> B[Finish]'

export function blocks(actions: BlockActions): Block[] {
  const chain = (editor: Editor) => editor.chain().focus()
  return [
    {
      name: 'Text',
      hint: 'Plain writing',
      words: 'paragraph plain',
      icon: Pilcrow,
      run: (e) => chain(e).setParagraph().run(),
    },
    {
      name: 'Heading 1',
      hint: 'A large section heading',
      words: 'title h1',
      icon: Heading1,
      run: (e) => chain(e).setHeading({ level: 1 }).run(),
    },
    {
      name: 'Heading 2',
      hint: 'A section heading',
      words: 'h2',
      icon: Heading2,
      run: (e) => chain(e).setHeading({ level: 2 }).run(),
    },
    {
      name: 'Heading 3',
      hint: 'A small heading',
      words: 'h3',
      icon: Heading3,
      run: (e) => chain(e).setHeading({ level: 3 }).run(),
    },
    {
      name: 'Bulleted list',
      hint: 'A simple list',
      words: 'ul unordered',
      icon: List,
      run: (e) => chain(e).toggleBulletList().run(),
    },
    {
      name: 'Numbered list',
      hint: 'A list in order',
      words: 'ol ordered',
      icon: ListOrdered,
      run: (e) => chain(e).toggleOrderedList().run(),
    },
    {
      name: 'To-do list',
      hint: 'Things to check off',
      words: 'task checkbox',
      icon: ListChecks,
      run: (e) => chain(e).toggleTaskList().run(),
    },
    {
      name: 'Quote',
      hint: 'Set a passage apart',
      words: 'blockquote',
      icon: Quote,
      run: (e) => chain(e).toggleBlockquote().run(),
    },
    {
      name: 'Code',
      hint: 'A block of code',
      words: 'snippet pre',
      icon: Code,
      run: (e) => chain(e).toggleCodeBlock().run(),
    },
    {
      name: 'Divider',
      hint: 'A line between sections',
      words: 'rule separator hr',
      icon: Minus,
      run: (e) => chain(e).setHorizontalRule().run(),
    },
    {
      name: 'Table',
      hint: 'Rows and columns',
      words: 'grid',
      icon: Table,
      run: (e) =>
        chain(e).insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(),
    },
    {
      name: 'Diagram',
      hint: 'A flowchart or sequence, from text',
      words: 'mermaid flowchart chart sequence',
      icon: Workflow,
      run: (e) =>
        chain(e)
          .insertContent({
            type: 'codeBlock',
            attrs: { language: 'mermaid' },
            content: [{ type: 'text', text: DIAGRAM }],
          })
          .run(),
    },
    {
      name: 'Image',
      hint: 'Upload a picture',
      words: 'picture photo',
      icon: ImageIcon,
      run: actions.pickImage,
    },
    {
      name: 'File',
      hint: 'Attach any file',
      words: 'attachment upload',
      icon: FileUp,
      run: actions.pickFile,
    },
    {
      name: 'Link to page',
      hint: 'Point at another page',
      words: 'mention wiki',
      icon: Link2,
      run: actions.pickPage,
    },
  ]
}

export function matching(all: Block[], query: string) {
  const needle = query.toLowerCase()
  return all.filter((block) =>
    `${block.name} ${block.words}`.toLowerCase().includes(needle),
  )
}

export type SlashView = Pick<
  SuggestionProps<Block, Block>,
  'items' | 'command' | 'clientRect'
> & { key: number }

/** The menu's open state, which the React menu reads; the extension only reports it. */
export type SlashEvents = {
  show: (view: SlashView | null) => void
  key: (event: KeyboardEvent) => boolean
}

let opened = 0

/** `/` anywhere opens the block menu. */
export function SlashCommands(items: () => Block[], events: SlashEvents) {
  return Extension.create({
    name: 'slashCommands',
    addProseMirrorPlugins() {
      return [
        Suggestion<Block, Block>({
          editor: this.editor,
          pluginKey: new PluginKey('slashCommands'),
          char: '/',
          startOfLine: false,
          allowSpaces: false,
          items: ({ query }) => matching(items(), query),
          command: ({
            editor,
            range,
            props,
          }: {
            editor: Editor
            range: Range
            props: Block
          }) => {
            editor.chain().focus().deleteRange(range).run()
            props.run(editor)
          },
          render: () => ({
            onStart: (props) => {
              opened += 1
              events.show({ ...view(props), key: opened })
            },
            onUpdate: (props) => events.show({ ...view(props), key: opened }),
            onKeyDown: ({ event }) => events.key(event),
            onExit: () => events.show(null),
          }),
        }),
      ]
    },
  })
}

function view(props: SuggestionProps<Block, Block>) {
  return {
    items: props.items,
    command: props.command,
    clientRect: props.clientRect ?? null,
  }
}
