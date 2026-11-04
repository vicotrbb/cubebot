import {
  NodeViewContent,
  NodeViewWrapper,
  type NodeViewProps,
} from '@tiptap/react'
import { useState } from 'react'
import { Diagram } from '../../components/Diagram'

/** A code block, or a diagram when fenced `mermaid`: the drawing, with its source one click away. */
export function CodeView({ node, editor }: NodeViewProps) {
  const language = node.attrs.language as string | null
  const [editing, setEditing] = useState(!node.textContent)
  if (language !== 'mermaid')
    return (
      <NodeViewWrapper as="pre" className="kb-code" data-language={language}>
        <NodeViewContent<'code'> as="code" />
      </NodeViewWrapper>
    )
  const open = editing && editor.isEditable
  return (
    <NodeViewWrapper className="kb-diagram" data-editing={open}>
      <Diagram source={node.textContent} />
      {editor.isEditable && (
        <button
          type="button"
          className="btn btn--sm kb-diagram__toggle"
          contentEditable={false}
          onClick={() => setEditing(!editing)}
        >
          {open ? 'Done' : 'Edit diagram'}
        </button>
      )}
      <pre className="kb-code kb-diagram__source">
        <NodeViewContent<'code'> as="code" />
      </pre>
    </NodeViewWrapper>
  )
}
