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
})
