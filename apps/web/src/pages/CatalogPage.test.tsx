import { fireEvent, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Route, Routes } from 'react-router-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { CaptionJobResult, CatalogTrack, CaptionView, Model } from '../api/types'
import {
  jsonResponse,
  makeCatalog,
  makeCatalogTrack,
  makeModel,
  mockFetch,
  renderWithProviders,
} from '../test/utils'
import { CatalogPage } from './CatalogPage'

const MOSS = makeModel({
  id: 'm1',
  key: 'moss-8b',
  name: 'MOSS-Music 8B',
  kind: 'captioner',
  engine: 'caption',
})
const ACE = makeModel({
  id: 'm2',
  key: 'ace-cap',
  name: 'ACE-Step Captioner',
  kind: 'captioner',
  engine: 'caption',
  installed_nodes: [],
})
const GENERATOR = makeModel({ id: 'm3', key: 'gen', name: 'Generator', kind: 'generator' })

function caption(overrides: Partial<CaptionView> = {}): CaptionView {
  return {
    model_key: 'moss-8b',
    seq: 1,
    mood: 'Warm',
    instruments: 'Piano',
    vocals: 'Instrumental',
    caption: 'A mellow lo-fi piece',
    source: 'model',
    source_ref: null,
    created_at: '',
    can_undo: false,
    can_redo: false,
    ...overrides,
  }
}

const tracks: CatalogTrack[] = [
  makeCatalogTrack({
    id: 't1',
    filename: 'lofi_jazz_001.mp3',
    artifact_id: 'art-1',
    duration_s: 192,
    tempo_bpm: 78,
    key_name: 'A minor',
    estimates: {
      tempo: { bpm: 78, confidence: 0.8, alternatives: [] },
      key: { key: 'A minor', confidence: 0.4 },
    },
    current_model_key: 'moss-8b',
    caption: caption(),
  }),
  makeCatalogTrack({
    id: 't2',
    filename: 'late_bus_home.mp3',
    artifact_id: 'art-2',
    duration_s: 298,
    estimates: {
      tempo: { bpm: null, confidence: 0.2, alternatives: [82, 164] },
      key: null,
    },
    voice_warning: true,
  }),
  makeCatalogTrack({
    id: 't3',
    filename: 'rainy_window.mp3',
    artifact_id: 'art-3',
    duration_s: 167,
    captioning_model_key: 'moss-8b',
    estimates: { tempo: { bpm: 70, confidence: 0.1, alternatives: [] }, key: null },
  }),
  makeCatalogTrack({ id: 't4', filename: 'night_market.mp3', artifact_id: 'art-4', duration_s: 220 }),
  makeCatalogTrack({ id: 't5', filename: 'broken.mp3', artifact_id: 'art-5', analysis_status: 'failed' }),
]

function setup({
  models = [MOSS, ACE, GENERATOR] as Model[],
  list = tracks,
  captionResult = { job_id: 'job-1', queued: ['t1', 't2'], skipped: ['t3'] } as CaptionJobResult,
  extraRoutes = {} as Record<string, unknown>,
} = {}) {
  const calls: { key: string; body: unknown }[] = []
  const fetchMock = mockFetch({
    'GET /catalogs/catalog-1': makeCatalog({ name: 'Kho nhạc Lo-fi', genre_name: 'Lofi', track_count: list.length }),
    'GET /catalogs/catalog-1/tracks': list,
    'GET /models': models,
    'POST /catalogs/catalog-1/caption-jobs': (init: RequestInit | undefined) => {
      calls.push({ key: 'caption', body: JSON.parse(init?.body as string) })
      return jsonResponse(captionResult, 201)
    },
    ...extraRoutes,
  })
  renderWithProviders(
    <Routes>
      <Route path="/catalogs/:id" element={<CatalogPage />} />
    </Routes>,
    { route: '/catalogs/catalog-1' },
  )
  return { calls, fetchMock, user: userEvent.setup() }
}

const rowOf = (name: string) => screen.getByText(name).closest('tr') as HTMLElement

afterEach(() => vi.restoreAllMocks())

describe('CatalogPage', () => {
  it('shows the header and one row per track with the right status', async () => {
    setup()
    expect(await screen.findByRole('heading', { name: 'Kho nhạc Lo-fi' })).toBeInTheDocument()
    expect(screen.getByText('Genre: Lofi')).toBeInTheDocument()
    await screen.findByText('lofi_jazz_001.mp3')
    expect(within(rowOf('lofi_jazz_001.mp3')).getByText('Có mô tả')).toBeInTheDocument()
    expect(within(rowOf('late_bus_home.mp3')).getByText('Mới tải lên')).toBeInTheDocument()
    expect(within(rowOf('rainy_window.mp3')).getByText('Đang tạo mô tả…')).toBeInTheDocument()
    expect(within(rowOf('broken.mp3')).getByText('Lỗi đo')).toBeInTheDocument()
    expect(within(rowOf('lofi_jazz_001.mp3')).getByText('MOSS-Music 8B')).toBeInTheDocument()
    expect(within(rowOf('night_market.mp3')).getByText('Chưa chọn')).toBeInTheDocument()
    expect(within(rowOf('late_bus_home.mp3')).getByText(/train 4 phút đầu/)).toBeInTheDocument()
    expect(within(rowOf('lofi_jazz_001.mp3')).queryByText(/train 4 phút đầu/)).not.toBeInTheDocument()
    expect(within(rowOf('late_bus_home.mp3')).getByText('Có thể có giọng')).toBeInTheDocument()
    const audio = rowOf('lofi_jazz_001.mp3').querySelector('audio') as HTMLAudioElement
    expect(audio.getAttribute('src')).toContain('/artifacts/art-1/content')
    expect(audio).toHaveAttribute('preload', 'none')
  })

  it('shows "Đang đo" for pending analysis', async () => {
    setup({ list: [makeCatalogTrack({ id: 'p', filename: 'p.mp3', analysis_status: 'pending' })] })
    expect(await screen.findByText('Đang đo')).toBeInTheDocument()
  })

  it('shows --:-- for a track without duration and never prints "null" for a missing key', async () => {
    setup({
      list: [
        makeCatalogTrack({
          id: 'n',
          filename: 'n.mp3',
          duration_s: null,
          tempo_source: null,
          key_source: null,
          estimates: { tempo: null, key: { key: null, confidence: 0 } },
        }),
      ],
    })
    await screen.findByText('n.mp3')
    const row = rowOf('n.mp3')
    expect(within(row).getByText('--:--')).toBeInTheDocument()
    expect(within(row).queryByText(/train 4 phút đầu/)).not.toBeInTheDocument()
    expect(within(row).getByText('Máy: không đủ căn cứ')).toBeInTheDocument()
    expect(within(row).queryByText(/null/)).not.toBeInTheDocument()
  })

  it('links to the queue while a caption job is running', async () => {
    setup()
    await screen.findByText('lofi_jazz_001.mp3')
    expect(screen.getByRole('link', { name: 'Hàng đợi' })).toHaveAttribute('href', '/queue')
  })

  it('shows no caption notice when nothing is being captioned', async () => {
    setup({ list: [makeCatalogTrack({ id: 'q', filename: 'q.mp3' })] })
    await screen.findByText('q.mp3')
    expect(screen.queryByRole('link', { name: 'Hàng đợi' })).not.toBeInTheDocument()
  })

  it('shows the machine reading in the three cases', async () => {
    setup()
    await screen.findByText('lofi_jazz_001.mp3')
    expect(within(rowOf('lofi_jazz_001.mp3')).getByText('Máy: 78 (tin cậy cao)')).toBeInTheDocument()
    expect(within(rowOf('lofi_jazz_001.mp3')).getByText('Máy: A minor (tin cậy khá)')).toBeInTheDocument()
    expect(within(rowOf('late_bus_home.mp3')).getByText('Máy: 82 hoặc 164')).toBeInTheDocument()
    expect(within(rowOf('late_bus_home.mp3')).getByText('Máy: không đủ căn cứ')).toBeInTheDocument()
    expect(within(rowOf('rainy_window.mp3')).getByText('Máy: 70 (tin cậy thấp)')).toBeInTheDocument()
  })

  it('creates caption jobs for the ticked tracks and reports the skipped ones', async () => {
    const { calls, user } = setup()
    await screen.findByText('lofi_jazz_001.mp3')
    await user.click(within(rowOf('lofi_jazz_001.mp3')).getByRole('checkbox'))
    await user.click(within(rowOf('late_bus_home.mp3')).getByRole('checkbox'))
    expect(screen.getByText('Đã chọn 2 bài')).toBeInTheDocument()
    await user.selectOptions(screen.getByLabelText('Model tạo mô tả'), 'moss-8b')
    await user.click(screen.getByRole('button', { name: 'Tạo mô tả cho 2 bài' }))
    await waitFor(() => expect(calls).toEqual([{ key: 'caption', body: { model_key: 'moss-8b', track_ids: ['t1', 't2'] } }]))
    expect(
      await screen.findByText(/Đã xếp hàng 2 bài, bỏ qua 1 bài đã có mô tả của model này/),
    ).toBeInTheDocument()
    expect(screen.getByText(/Trong lúc tạo mô tả, nhạc mới tạo sẽ phải chờ/)).toBeInTheDocument()
    expect(screen.getByText('Đã chọn 0 bài')).toBeInTheDocument()
  })

  it('omits the wait warning when nothing was queued', async () => {
    const { user } = setup({ captionResult: { job_id: null, queued: [], skipped: ['t1'] } })
    await screen.findByText('lofi_jazz_001.mp3')
    await user.click(within(rowOf('lofi_jazz_001.mp3')).getByRole('checkbox'))
    await user.click(screen.getByRole('button', { name: 'Tạo mô tả cho 1 bài' }))
    expect(await screen.findByText(/Đã xếp hàng 0 bài, bỏ qua 1 bài/)).toBeInTheDocument()
    expect(screen.queryByText(/nhạc mới tạo sẽ phải chờ/)).not.toBeInTheDocument()
  })

  it('"Chọn N bài đầu" ticks the first N tracks; "Chọn tất cả" ticks all', async () => {
    const { user } = setup()
    await screen.findByText('lofi_jazz_001.mp3')
    const n = screen.getByLabelText('Số bài đầu')
    await user.clear(n)
    await user.type(n, '3')
    await user.click(screen.getByRole('button', { name: 'Chọn bài đầu' }))
    expect(screen.getByText('Đã chọn 3 bài')).toBeInTheDocument()
    expect(within(rowOf('rainy_window.mp3')).getByRole('checkbox')).toBeChecked()
    expect(within(rowOf('night_market.mp3')).getByRole('checkbox')).not.toBeChecked()
    await user.click(screen.getByRole('button', { name: 'Chọn tất cả' }))
    expect(screen.getByText('Đã chọn 5 bài')).toBeInTheDocument()
  })

  it('greys out uninstalled captioners and disables the button without an installed one', async () => {
    const { user } = setup({ models: [ACE, GENERATOR] })
    await screen.findByText('lofi_jazz_001.mp3')
    await user.click(within(rowOf('lofi_jazz_001.mp3')).getByRole('checkbox'))
    const menu = screen.getByLabelText('Model tạo mô tả')
    expect(within(menu).getByRole('option', { name: /ACE-Step Captioner.*Chưa cài trên máy/ })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Tạo mô tả cho 1 bài' })).toBeDisabled()
  })

  it('disables the menu when there is no captioner at all', async () => {
    setup({ models: [GENERATOR] })
    await screen.findByText('lofi_jazz_001.mp3')
    const menu = screen.getByLabelText('Model tạo mô tả')
    expect(menu).toBeDisabled()
    expect(within(menu).getByText('Chưa cài model mô tả trên máy render')).toBeInTheDocument()
  })

  it('uploads files one by one, reports a 409 and continues with the next file', async () => {
    const posted: string[] = []
    const { user } = setup({
      extraRoutes: {
        'POST /catalogs/catalog-1/tracks': (init: RequestInit | undefined) => {
          const name = ((init?.body as FormData).get('file') as File).name
          posted.push(name)
          if (name === 'a.mp3') {
            return jsonResponse(
              { error: { code: 'conflict', message: 'File này đã có trong catalog: old_a.mp3' } },
              409,
            )
          }
          return jsonResponse(makeCatalogTrack({ id: 'new', filename: name }), 201)
        },
      },
    })
    await screen.findByText('lofi_jazz_001.mp3')
    const input = screen.getByTestId('catalog-upload-input') as HTMLInputElement
    await user.upload(input, [new File(['x'], 'a.mp3'), new File(['y'], 'b.mp3')])
    expect(await screen.findByText('a.mp3: đã có trong catalog (old_a.mp3)')).toBeInTheDocument()
    await waitFor(() => expect(posted).toEqual(['a.mp3', 'b.mp3']))
  })

  it('rejects bad files client-side without a request', async () => {
    const posted: string[] = []
    setup({
      extraRoutes: {
        'POST /catalogs/catalog-1/tracks': () => {
          posted.push('x')
          return jsonResponse({}, 201)
        },
      },
    })
    await screen.findByText('lofi_jazz_001.mp3')
    const input = screen.getByTestId('catalog-upload-input') as HTMLInputElement
    fireEvent.change(input, { target: { files: [new File(['x'], 'notes.txt')] } })
    expect(await screen.findByText(/notes\.txt: Chỉ nhận file/)).toBeInTheDocument()
    expect(posted).toEqual([])
  })

  it('retries a failed analysis', async () => {
    const reanalyzed: string[] = []
    const { user } = setup({
      extraRoutes: {
        'POST /catalog-tracks/t5/reanalyze': () => {
          reanalyzed.push('t5')
          return jsonResponse(makeCatalogTrack({ id: 't5', analysis_status: 'pending' }))
        },
      },
    })
    await screen.findByText('broken.mp3')
    await user.click(within(rowOf('broken.mp3')).getByRole('button', { name: 'Thử lại' }))
    await waitFor(() => expect(reanalyzed).toEqual(['t5']))
  })

  it('deletes ticked tracks after confirmation, one request each', async () => {
    const deleted: string[] = []
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    const { user } = setup({
      extraRoutes: {
        'DELETE /catalog-tracks/t1': () => (deleted.push('t1'), new Response(null, { status: 204 })),
        'DELETE /catalog-tracks/t2': () => (deleted.push('t2'), new Response(null, { status: 204 })),
      },
    })
    await screen.findByText('lofi_jazz_001.mp3')
    await user.click(within(rowOf('lofi_jazz_001.mp3')).getByRole('checkbox'))
    await user.click(within(rowOf('late_bus_home.mp3')).getByRole('checkbox'))
    await user.click(screen.getByRole('button', { name: 'Xóa khỏi catalog' }))
    expect(confirm).toHaveBeenCalledWith('Xóa 2 bài khỏi catalog?')
    await waitFor(() => expect(deleted).toEqual(['t1', 't2']))
  })

  describe('inline edits', () => {
    const MOSS_DONE = makeCatalogTrack({
      id: 'e1',
      filename: 'edit_me.mp3',
      current_model_key: 'moss-8b',
      models_done: ['moss-8b'],
      caption: caption({ can_undo: true, can_redo: true }),
    })

    function setupEdit(track: CatalogTrack, extra: Record<string, unknown> = {}) {
      const bodies: { key: string; body: unknown }[] = []
      const rec = (key: string, res: unknown = {}) => (init: RequestInit | undefined) => {
        bodies.push({ key, body: init?.body ? JSON.parse(init.body as string) : null })
        return jsonResponse(res)
      }
      const view = setup({
        list: [track],
        extraRoutes: {
          'PATCH /catalog-tracks/e1': rec('patch'),
          'PUT /catalog-tracks/e1/captions/moss-8b': rec('put'),
          'POST /catalog-tracks/e1/captions/moss-8b/undo': rec('undo'),
          'POST /catalog-tracks/e1/captions/moss-8b/redo': rec('redo'),
          ...extra,
        },
      })
      return { ...view, bodies }
    }

    it('switching to a model not yet run patches, then shows empty cells and "Tạo"', async () => {
      const MOSS2 = makeModel({ id: 'm4', key: 'moss-2', name: 'Second', kind: 'captioner', engine: 'caption' })
      let patched = false
      const fresh = { ...MOSS_DONE, current_model_key: 'moss-2', caption: null }
      const { bodies, user } = setupEdit(MOSS_DONE, {
        'GET /models': [MOSS, MOSS2],
        'GET /catalogs/catalog-1/tracks': () => jsonResponse([patched ? fresh : MOSS_DONE]),
        'PATCH /catalog-tracks/e1': (init: RequestInit | undefined) => {
          patched = true
          bodies.push({ key: 'patch', body: JSON.parse(init?.body as string) })
          return jsonResponse(fresh)
        },
      })
      await screen.findByText('edit_me.mp3')
      const row = rowOf('edit_me.mp3')
      expect(within(row).getByRole('button', { name: 'Chạy lại' })).toBeInTheDocument()
      await user.selectOptions(within(row).getByRole('combobox'), 'moss-2')
      await waitFor(() => expect(bodies).toEqual([{ key: 'patch', body: { current_model_key: 'moss-2' } }]))
      expect(await within(row).findByRole('button', { name: 'Tạo' })).toBeInTheDocument()
      expect(within(row).getByText('Chưa có mô tả.')).toBeInTheDocument()
    })

    it('marks models already done with a dot', async () => {
      setupEdit(MOSS_DONE)
      await screen.findByText('edit_me.mp3')
      const select = within(rowOf('edit_me.mp3')).getByRole('combobox')
      expect(within(select).getByRole('option', { name: '● MOSS-Music 8B' })).toBeInTheDocument()
      expect(within(select).getByRole('option', { name: /^ACE-Step Captioner/ })).toBeDisabled()
    })

    it('re-run asks first; cancelling sends nothing, accepting forces the job', async () => {
      const { calls, user } = setupEdit(MOSS_DONE)
      await screen.findByText('edit_me.mp3')
      const btn = within(rowOf('edit_me.mp3')).getByRole('button', { name: 'Chạy lại' })
      const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false)
      await user.click(btn)
      expect(confirm).toHaveBeenCalledWith(
        'Chạy lại sẽ thay bản hiện tại, kể cả phần bạn đã sửa. Bấm Quay về để lấy lại.',
      )
      expect(calls).toEqual([])
      confirm.mockReturnValue(true)
      await user.click(btn)
      await waitFor(() =>
        expect(calls).toEqual([{ key: 'caption', body: { model_key: 'moss-8b', track_ids: ['e1'], force: true } }]),
      )
    })

    it('disables Chạy lại while the track is being captioned', async () => {
      setupEdit({ ...MOSS_DONE, captioning_model_key: 'moss-8b' })
      await screen.findByText('edit_me.mp3')
      expect(within(rowOf('edit_me.mp3')).getByRole('button', { name: 'Chạy lại' })).toBeDisabled()
    })

    it('Quay về and Tới call undo and redo', async () => {
      const { bodies, user } = setupEdit(MOSS_DONE)
      await screen.findByText('edit_me.mp3')
      await user.click(within(rowOf('edit_me.mp3')).getByRole('button', { name: 'Quay về' }))
      await user.click(within(rowOf('edit_me.mp3')).getByRole('button', { name: 'Tới' }))
      await waitFor(() => expect(bodies.map((b) => b.key)).toEqual(['undo', 'redo']))
    })

    it('editing Mood and pressing Enter PUTs all four fields', async () => {
      const { bodies, user } = setupEdit(MOSS_DONE)
      await screen.findByText('edit_me.mp3')
      await user.click(within(rowOf('edit_me.mp3')).getByRole('button', { name: /Sửa Mood/ }))
      const input = screen.getByRole('textbox', { name: /Mood/ })
      await user.clear(input)
      await user.type(input, 'Dreamy{Enter}')
      await waitFor(() =>
        expect(bodies).toEqual([
          {
            key: 'put',
            body: { mood: 'Dreamy', instruments: 'Piano', vocals: 'Instrumental', caption: 'A mellow lo-fi piece' },
          },
        ]),
      )
    })

    it('Esc cancels an edit', async () => {
      const { bodies, user } = setupEdit(MOSS_DONE)
      await screen.findByText('edit_me.mp3')
      await user.click(within(rowOf('edit_me.mp3')).getByRole('button', { name: /Sửa Mood/ }))
      await user.type(screen.getByRole('textbox', { name: /Mood/ }), 'x{Escape}')
      expect(bodies).toEqual([])
      expect(within(rowOf('edit_me.mp3')).getByRole('button', { name: /Sửa Mood/ })).toHaveTextContent('Warm')
    })

    it('Tempo accepts numbers only and sends null when emptied; Key sends free text', async () => {
      const { bodies, user } = setupEdit({ ...MOSS_DONE, tempo_bpm: 78 })
      await screen.findByText('edit_me.mp3')
      const row = rowOf('edit_me.mp3')
      await user.click(within(row).getByRole('button', { name: /Sửa Tempo/ }))
      const tempo = screen.getByRole('textbox', { name: /Tempo/ })
      await user.clear(tempo)
      await user.type(tempo, 'abc{Enter}')
      expect(await screen.findByText('Chỉ nhập số')).toBeInTheDocument()
      expect(bodies).toEqual([])
      await user.clear(tempo)
      await user.type(tempo, '{Enter}')
      await waitFor(() => expect(bodies).toEqual([{ key: 'patch', body: { tempo_bpm: null } }]))
      await user.click(within(row).getByRole('button', { name: /Sửa Key/ }))
      await user.type(screen.getByRole('textbox', { name: /Key/ }), 'C major{Enter}')
      await waitFor(() => expect(bodies[1]).toEqual({ key: 'patch', body: { key_name: 'C major' } }))
    })

    it('caption fields are locked until a model is chosen', async () => {
      setupEdit(makeCatalogTrack({ id: 'e1', filename: 'edit_me.mp3' }))
      await screen.findByText('edit_me.mp3')
      const btn = within(rowOf('edit_me.mp3')).getByRole('button', { name: /Sửa Mood/ })
      expect(btn).toBeDisabled()
      expect(btn).toHaveAttribute('title', 'Chọn model trước')
    })

    it('shows a save error inline under the cell', async () => {
      const { user } = setupEdit(MOSS_DONE, {
        'PUT /catalog-tracks/e1/captions/moss-8b': () =>
          jsonResponse({ error: { code: 'bad', message: 'Không lưu được' } }, 422),
      })
      await screen.findByText('edit_me.mp3')
      await user.click(within(rowOf('edit_me.mp3')).getByRole('button', { name: /Sửa Mood/ }))
      await user.type(screen.getByRole('textbox', { name: /Mood/ }), 'x{Enter}')
      expect(await screen.findByText(/Không lưu được/)).toBeInTheDocument()
      expect(screen.getByRole('textbox', { name: /Mood/ })).toHaveValue('Warmx')
    })
  })
})
