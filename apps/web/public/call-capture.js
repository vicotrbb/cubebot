// Runs on the audio thread: averages the microphone down to 16 kHz mono PCM16 and posts it in
// 100 ms frames. AudioContext rates vary by device, so the rate conversion cannot be left out.
const FRAME = 1600

class CallCapture extends globalThis.AudioWorkletProcessor {
  frame = new Int16Array(FRAME)
  filled = 0
  phase = 0
  sum = 0
  count = 0

  process(inputs) {
    const channel = inputs[0]?.[0]
    if (!channel) return true
    const step = 16000 / globalThis.sampleRate
    for (const sample of channel) {
      this.sum += sample
      this.count += 1
      this.phase += step
      if (this.phase >= 1) {
        this.phase -= 1
        const average = Math.max(-1, Math.min(1, this.sum / this.count))
        this.frame[this.filled] = average * 32767
        this.filled += 1
        this.sum = 0
        this.count = 0
        if (this.filled === FRAME) {
          this.port.postMessage(this.frame.buffer.slice(0))
          this.filled = 0
        }
      }
    }
    return true
  }
}

globalThis.registerProcessor('call-capture', CallCapture)
