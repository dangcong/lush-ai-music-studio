import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, vi } from 'vitest'

// jsdom does not implement media playback; emulate the events a browser would fire.
window.HTMLMediaElement.prototype.play = vi.fn(function (this: HTMLMediaElement) {
  this.dispatchEvent(new Event('play'))
  return Promise.resolve()
})
window.HTMLMediaElement.prototype.pause = vi.fn(function (this: HTMLMediaElement) {
  this.dispatchEvent(new Event('pause'))
})
window.HTMLMediaElement.prototype.load = vi.fn()

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})
