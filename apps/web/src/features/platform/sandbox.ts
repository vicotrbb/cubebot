import type { Installation } from '../../api/types'

/** Only `kvm` is a virtual machine; a container is never called one, whatever runtime class it names. */
export function sandboxLabel(installation: Installation): string {
  switch (installation.sandbox_runtime) {
    case 'kvm':
      return 'KVM virtual machine'
    case 'container':
      return installation.sandbox_runtime_class
        ? `container (${installation.sandbox_runtime_class} runtime)`
        : 'container'
    default:
      return 'none'
  }
}
