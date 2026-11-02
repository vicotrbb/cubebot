import test from 'node:test'
import assert from 'node:assert/strict'
import { sandboxLabel } from './sandbox'

const installation = {
  name: 'Cubebot',
  terminal_enabled: true,
  computer_use_enabled: true,
  controller: 'docker' as const,
}

test('only the kvm runtime is called a virtual machine', () => {
  const kvm = { ...installation, sandbox_runtime: 'kvm' as const }
  assert.equal(sandboxLabel(kvm), 'KVM virtual machine')
  for (const sandbox_runtime_class of [null, 'kata', 'vm']) {
    const label = sandboxLabel({
      ...installation,
      sandbox_runtime: 'container',
      sandbox_runtime_class,
    })
    assert.match(label, /^container/)
    assert.doesNotMatch(label, /virtual machine/i)
  }
  assert.equal(sandboxLabel({ ...installation, sandbox_runtime: null }), 'none')
})
