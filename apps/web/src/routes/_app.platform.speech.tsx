import { createFileRoute } from '@tanstack/react-router'
import { PlatformSpeech } from '../features/call/SpeechSettings'

export const Route = createFileRoute('/_app/platform/speech')({
  component: PlatformSpeech,
})
