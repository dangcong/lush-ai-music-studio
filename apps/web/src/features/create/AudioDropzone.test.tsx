import { fireEvent, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { jsonResponse, mockFetch, renderWithProviders } from '../../test/utils'
import { AudioDropzone, checkAudioFile } from './AudioDropzone'

const MB = 1024 * 1024

function fileOfSize(name: string, size: number): File {
  const file = new File(['x'], name)
  Object.defineProperty(file, 'size', { value: size })
  return file
}

function setup(value = '') {
  const onChange = vi.fn()
  const fetchMock = mockFetch({
    'POST /audio-inputs': () =>
      jsonResponse(
        { id: 'in-9', filename: 'song.flac', content_type: 'audio/flac', size_bytes: 2 * MB, sha256: 'aa', created_at: '' },
        201,
      ),
    'GET /audio-inputs/in-old': {
      id: 'in-old',
      filename: 'earlier.wav',
      content_type: 'audio/wav',
      size_bytes: 3 * MB,
      sha256: 'bb',
      created_at: '',
    },
  })
  renderWithProviders(<AudioDropzone label="Reference Audio" value={value} onChange={onChange} />)
  return { onChange, fetchMock }
}

describe('checkAudioFile', () => {
  it('accepts wav, mp3 and flac up to 50 MB', () => {
    expect(checkAudioFile({ name: 'a.WAV', size: 10 })).toBeNull()
    expect(checkAudioFile({ name: 'a.mp3', size: 50 * MB })).toBeNull()
    expect(checkAudioFile({ name: 'a.flac', size: 10 })).toBeNull()
  })

  it('rejects other types, empty and oversized files', () => {
    expect(checkAudioFile({ name: 'a.ogg', size: 10 })).toMatch(/Chỉ nhận file WAV, MP3, FLAC/)
    expect(checkAudioFile({ name: 'mp3', size: 10 })).toMatch(/Chỉ nhận file/)
    expect(checkAudioFile({ name: 'a.wav', size: 0 })).toBe('File rỗng.')
    expect(checkAudioFile({ name: 'a.wav', size: 50 * MB + 1 })).toMatch(/Tối đa 50 MB/)
  })
})

describe('AudioDropzone', () => {
  it('rejects a wrong extension without uploading', async () => {
    const { onChange, fetchMock } = setup()
    // applyAccept: false = the user forced a non-audio file through the picker.
    const user = userEvent.setup({ applyAccept: false })
    await user.upload(screen.getByLabelText('Chọn file Reference Audio'), new File(['x'], 'notes.txt'))

    expect(await screen.findByRole('alert')).toHaveTextContent('Chỉ nhận file WAV, MP3, FLAC.')
    expect(fetchMock).not.toHaveBeenCalled()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('rejects a dropped file over 50 MB without uploading', async () => {
    const { onChange, fetchMock } = setup()
    fireEvent.drop(screen.getByRole('button', { name: 'Reference Audio' }), {
      dataTransfer: { files: [fileOfSize('long-mix.wav', 51 * MB)] },
    })

    expect(await screen.findByRole('alert')).toHaveTextContent('Tối đa 50 MB')
    expect(fetchMock).not.toHaveBeenCalled()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('uploads a dropped file and reports its id', async () => {
    const { onChange } = setup()
    fireEvent.drop(screen.getByRole('button', { name: 'Reference Audio' }), {
      dataTransfer: { files: [new File(['fLaC'], 'song.flac')] },
    })
    await waitFor(() => expect(onChange).toHaveBeenCalledWith('in-9'))
  })

  it('shows the API error when the upload is refused', async () => {
    const onChange = vi.fn()
    mockFetch({
      'POST /audio-inputs': () =>
        jsonResponse({ error: { code: 'validation', message: 'File không phải audio hợp lệ' } }, 422),
    })
    renderWithProviders(<AudioDropzone label="Source Audio" value="" onChange={onChange} />)
    await userEvent.setup().upload(screen.getByLabelText('Chọn file Source Audio'), new File(['x'], 'fake.mp3'))

    expect(await screen.findByRole('alert')).toHaveTextContent('File không phải audio hợp lệ')
    expect(onChange).not.toHaveBeenCalled()
  })

  it('shows a file restored by id, with a preview and a way to remove it', async () => {
    const { onChange } = setup('in-old')
    expect(await screen.findByText('earlier.wav')).toBeInTheDocument()
    expect(screen.getByLabelText('Nghe thử Reference Audio')).toHaveAttribute(
      'src',
      '/api/v1/artifacts/in-old/content',
    )
    await userEvent.setup().click(screen.getByRole('button', { name: 'Bỏ file Reference Audio' }))
    expect(onChange).toHaveBeenCalledWith('')
  })
})
