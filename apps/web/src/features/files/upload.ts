import { ApiError } from '../../api/client'
import type { components } from '../../api/schema'

type FileEntry = components['schemas']['FileEntry']

export class NameTaken extends Error {}

export async function uploadFile(
  file: File,
  parent: string | undefined,
  name = file.name,
): Promise<FileEntry> {
  const query = new URLSearchParams({ name, request_id: crypto.randomUUID() })
  if (parent) query.set('parent', parent)
  const response = await fetch(`/api/files?${query}`, {
    method: 'POST',
    body: file,
    credentials: 'same-origin',
    headers: { 'content-type': file.type || 'application/octet-stream' },
  })
  if (response.status === 507)
    throw new Error(
      'The installation is out of file storage. Delete files or ask an administrator.',
    )
  if (response.status === 413)
    throw new Error(
      `${file.name} is larger than this installation’s upload limit. An administrator can raise it under Platform › General.`,
    )
  if (response.status === 409)
    throw new NameTaken(`A file named ${name} is already in this folder.`)
  if (!response.ok) throw new ApiError(response.status)
  return (await response.json()) as FileEntry
}
