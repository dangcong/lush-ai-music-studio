import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Profiler } from 'react'
import { describe, expect, it } from 'vitest'
import type { GenerationRequest } from '../../api/types'
import {
  jsonResponse,
  makeGenre,
  makeJob,
  makeLora,
  makeModel,
  makePreset,
  makeUser,
  mockFetch,
  renderWithProviders,
} from '../../test/utils'
import { GenerateForm } from './GenerateForm'

const genres = [
  makeGenre({
    id: 'g-jazz',
    name: 'Jazz',
    slug: 'jazz',
    instruments: ['piano', 'upright bass'],
    moods: ['warm'],
    duration_min: 60,
    duration_max: 90,
    bpm_min: 70,
    bpm_max: 110,
  }),
  makeGenre({ id: 'g-lofi', name: 'Lo-fi', slug: 'lofi', sort_order: 1 }),
  makeGenre({ id: 'g-bebop', parent_id: 'g-jazz', name: 'Bebop', slug: 'bebop' }),
  makeGenre({ id: 'g-chillhop', parent_id: 'g-lofi', name: 'Chillhop', slug: 'chillhop' }),
]

const models = [
  makeModel({ id: 'm-a', name: 'Model A' }),
  makeModel({ id: 'm-b', name: 'Model B', installed_nodes: [{ node_id: 'n2', node_name: 'node-2', online: false }] }),
  makeModel({ id: 'm-off', name: 'Model Off', status: 'disabled' }),
]

// Every Style is a preset backed by an approved Train Style.
const baseLoras = [
  makeLora({ id: 'l-smoky', name: 'Smoky Style', base_model_id: 'm-b' }),
  makeLora({ id: 'l-tape', name: 'Tape Style', base_model_id: 'm-a' }),
]

const presets = [
  makePreset({
    id: 'p-smoky',
    genre_id: 'g-jazz',
    name: 'Smoky Club',
    kind: 'trained',
    lora_id: 'l-smoky',
    lora_scale: 1,
    model_id: 'm-b',
    duration_min: 120,
    duration_max: 180,
    bpm: 84,
    instruments: ['saxophone'],
    moods: ['smoky'],
    params: { inference_steps: 40 },
  }),
  makePreset({ id: 'p-tape', genre_id: 'g-lofi', name: 'Tape Hiss', kind: 'trained', lora_id: 'l-tape', lora_scale: 1 }),
]

function setup(
  loras: unknown[] = baseLoras,
  modelList = models,
  role: 'admin' | 'member' = 'member',
  presetList = presets,
  commits?: { n: number },
  extraRoutes: Record<string, unknown> = {},
) {
  const posted: GenerationRequest[] = []
  const uploads: File[] = []
  mockFetch({
    'GET /genres': genres,
    'GET /presets': presetList,
    'GET /models': modelList,
    'POST /audio-inputs': (init: RequestInit | undefined) => {
      const file = (init?.body as FormData).get('file') as File
      uploads.push(file)
      return jsonResponse(
        {
          id: `in-${uploads.length}`,
          filename: file.name,
          content_type: 'audio/mpeg',
          size_bytes: file.size,
          sha256: 'aa',
          created_at: '2026-01-01T00:00:00.000Z',
        },
        201,
      )
    },
    'GET /loras': loras,
    ...extraRoutes,
    'POST /generations': (init: RequestInit | undefined) => {
      posted.push(JSON.parse(init?.body as string) as GenerationRequest)
      return jsonResponse(makeJob(), 201)
    },
  })
  renderWithProviders(
    <Profiler id="gf" onRender={() => commits && commits.n++}>
      <GenerateForm />
    </Profiler>,
    { user: makeUser({ role }) },
  )
  return { posted, uploads, user: userEvent.setup() }
}

function optionNames(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole('option')
    .map((o) => o.textContent ?? '')
}

describe('GenerateForm', () => {
  it('lists only active models and selects one by default', async () => {
    setup()
    const model = await screen.findByLabelText('Model')
    expect(optionNames(model)).toEqual(['Model A (1)', 'Model B (1)'])
    await waitFor(() => expect(model).toHaveValue('m-a'))
  })

  it('settles without a render loop when only a captioner model exists', async () => {
    const commits = { n: 0 }
    setup(baseLoras, [makeModel({ id: 'm-cap', name: 'Captioner X', kind: 'captioner' })], 'member', presets, commits)
    const model = await screen.findByLabelText('Model')
    expect(optionNames(model)).toEqual(['— Chưa có model nào đang bật —'])
    await new Promise((r) => setTimeout(r, 300))
    expect(commits.n).toBeLessThan(30)
  })

  it('does not list captioner models', async () => {
    setup(baseLoras, [...models, makeModel({ id: 'm-cap', name: 'Captioner X', kind: 'captioner' })])
    const model = await screen.findByLabelText('Model')
    expect(optionNames(model)).toEqual(['Model A (1)', 'Model B (1)'])
  })

  it('filters subgenres and presets by the selected genre', async () => {
    const { user } = setup()
    const genre = await screen.findByLabelText('Genre')
    const subgenre = screen.getByLabelText('Subgenre')
    expect(optionNames(genre)).toEqual(['— Không chọn —', 'Jazz', 'Lo-fi'])
    expect(subgenre).toBeDisabled()

    await user.selectOptions(genre, 'g-jazz')
    expect(subgenre).toBeEnabled()
    expect(optionNames(subgenre)).toEqual(['— Không chọn —', 'Bebop'])
    expect(optionNames(screen.getByLabelText('Style'))).toEqual(['— Không dùng Style —', 'Smoky Club'])

    await user.selectOptions(genre, 'g-lofi')
    expect(optionNames(subgenre)).toEqual(['— Không chọn —', 'Chillhop'])
    expect(optionNames(screen.getByLabelText('Style'))).toEqual(['— Không dùng Style —', 'Tape Hiss'])
  })

  it('applies genre defaults and suggests its instruments and moods', async () => {
    const { user } = setup()
    await user.selectOptions(await screen.findByLabelText('Genre'), 'g-jazz')

    // Selecting a genre jumps to the upper end of its 60–90 s range.
    expect(screen.getByLabelText('Duration')).toHaveValue('90')
    expect(screen.getByText('Gợi ý cho Jazz: 60–90 giây.')).toBeInTheDocument()
    const piano = screen.getByRole('button', { name: 'piano' })
    expect(piano).toHaveAttribute('aria-pressed', 'false')
    await user.click(piano)
    expect(piano).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'warm' })).toBeInTheDocument()
  })

  it('pre-fills from a preset but leaves every field editable', async () => {
    const { user } = setup()
    await user.selectOptions(await screen.findByLabelText('Genre'), 'g-jazz')
    await user.selectOptions(screen.getByLabelText('Style'), 'p-smoky')

    expect(screen.getByLabelText('Model')).toHaveValue('m-b')
    expect(screen.getByLabelText('Duration')).toHaveValue('180')
    expect(screen.getByRole('button', { name: 'saxophone' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'smoky' })).toHaveAttribute('aria-pressed', 'true')

    await user.click(screen.getByRole('button', { name: 'Nâng cao' }))
    const bpm = screen.getByLabelText('BPM')
    expect(bpm).toHaveValue(84)
    expect(screen.getByLabelText('Inference Steps')).toHaveValue(40)

    await user.clear(bpm)
    await user.type(bpm, '90')
    expect(bpm).toHaveValue(90)
    await user.selectOptions(screen.getByLabelText('Model'), 'm-a')
    expect(screen.getByLabelText('Model')).toHaveValue('m-a')
  })

  it('warns when no online node has the model, without blocking submit', async () => {
    const { user } = setup()
    const model = await screen.findByLabelText('Model')
    await waitFor(() => expect(model).toHaveValue('m-a'))
    expect(screen.queryByText(/không có Render Node nào online/i)).not.toBeInTheDocument()

    await user.selectOptions(model, 'm-b')
    expect(screen.getByText(/không có Render Node nào online/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Generate' })).toBeEnabled()
  })

  it('explains that BPM / Key / Time Signature are only guidance', async () => {
    const { user } = setup()
    await user.click(await screen.findByRole('button', { name: 'Nâng cao' }))
    expect(screen.getByText(/hướng dẫn cho model, không bảo đảm chính xác/)).toBeInTheDocument()
    expect(screen.queryByLabelText('Train Style Scale')).not.toBeInTheDocument()
  })

  describe('Train Style through a Style', () => {
    const loras = [
      makeLora({ id: 'l-ok', name: 'Lofi Style', base_model_id: 'm-a', status: 'approved' }),
      makeLora({ id: 'l-eval', name: 'Jazz Style', base_model_id: 'm-a', status: 'evaluation' }),
      makeLora({ id: 'l-old', name: 'Old Style', base_model_id: 'm-a', status: 'archived' }),
    ]
    const loraPresets = [
      makePreset({ id: 'p-plain', name: 'Plain' }),
      makePreset({ id: 'p-ok', name: 'Lofi Tape', kind: 'trained', model_id: 'm-a', lora_id: 'l-ok', lora_scale: 0.8 }),
      makePreset({ id: 'p-eval', name: 'Jazz Trial', kind: 'trained', model_id: 'm-a', lora_id: 'l-eval', lora_scale: 1 }),
      makePreset({ id: 'p-old', name: 'Old One', kind: 'trained', model_id: 'm-a', lora_id: 'l-old', lora_scale: 1 }),
    ]

    it('has no separate Train Style picker: the Style is the only way to choose a Train Style', async () => {
      const { user } = setup(loras, models, 'admin', loraPresets)
      await screen.findByLabelText('Style')
      expect(screen.queryByLabelText('Train Style')).not.toBeInTheDocument()
      await user.click(screen.getByRole('button', { name: 'Nâng cao' }))
      expect(screen.queryByLabelText('Train Style Scale')).not.toBeInTheDocument()
    })

    it('a preset without a Train Style is not a Style and is never offered', async () => {
      setup(loras, models, 'admin', loraPresets)
      const select = await screen.findByLabelText('Style')
      await waitFor(() => expect(optionNames(select)).toContain('Lofi Tape'))
      expect(optionNames(select)).not.toContain('Plain')
    })

    it('members are only offered presets whose Train Style is approved', async () => {
      setup(loras, models, 'member', loraPresets)
      const select = await screen.findByLabelText('Style')
      await waitFor(() =>
        expect(optionNames(select)).toEqual(['— Không dùng Style —', 'Lofi Tape']),
      )
    })

    it('admins are also offered presets whose Train Style is under evaluation, and are told so', async () => {
      const { user } = setup(loras, models, 'admin', loraPresets)
      const select = await screen.findByLabelText('Style')
      await waitFor(() => expect(optionNames(select)).toContain('Jazz Trial'))
      expect(optionNames(select)).not.toContain('Old One')
      await user.selectOptions(select, 'p-eval')
      expect(screen.getByText(/Dùng Train Style “Jazz Style” \(đang đánh giá, chỉ admin nghe thử\)/)).toBeInTheDocument()
    })

    it('a preset sends its Train Style and scale; changing the model drops the Train Style', async () => {
      const { user, posted } = setup(loras, models, 'member', loraPresets)
      await user.type(await screen.findByLabelText('Prompt'), 'Warm jazz piano')
      const select = screen.getByLabelText('Style')
      await waitFor(() => expect(optionNames(select)).toContain('Lofi Tape'))
      await user.selectOptions(select, 'p-ok')
      expect(screen.getByText(/Dùng Train Style “Lofi Style”/)).toBeInTheDocument()

      const generate = screen.getByRole('button', { name: 'Generate' })
      await user.click(generate)
      await waitFor(() => expect(posted).toHaveLength(1))
      expect(posted[0]).toMatchObject({ preset_id: 'p-ok', model_id: 'm-a', lora_id: 'l-ok', lora_scale: 0.8 })

      await user.selectOptions(screen.getByLabelText('Model'), 'm-b')
      await waitFor(() => expect(generate).toBeEnabled())
      await user.click(generate)
      await waitFor(() => expect(posted).toHaveLength(2))
      expect(posted[1]).toMatchObject({ model_id: 'm-b', lora_id: null, lora_scale: null })
    })
  })

  it('requires a prompt before sending anything', async () => {
    const { user, posted } = setup()
    await user.click(await screen.findByRole('button', { name: 'Generate' }))
    expect(await screen.findByText('Hãy nhập Prompt.')).toBeInTheDocument()
    expect(posted).toHaveLength(0)
  })

  it('posts a body matching the contract, with a new idempotency_key per click', async () => {
    const { user, posted } = setup()
    await user.type(await screen.findByLabelText('Prompt'), '  Warm jazz piano  ')
    await user.type(screen.getByLabelText('Tiêu đề'), 'Rainy night')
    await user.selectOptions(screen.getByLabelText('Genre'), 'g-jazz')
    await user.selectOptions(screen.getByLabelText('Subgenre'), 'g-bebop')
    await user.click(screen.getByRole('button', { name: 'piano' }))
    await user.type(screen.getByLabelText('Thêm Moods'), 'late night{Enter}')
    await user.click(screen.getByRole('button', { name: 'Nâng cao' }))
    await user.type(screen.getByLabelText('BPM'), '80')
    await user.type(screen.getByLabelText('Key'), 'A minor')
    await user.type(screen.getByLabelText('Seed'), '1234')
    await user.type(screen.getByLabelText('Guidance Scale'), '7.5')

    const generate = screen.getByRole('button', { name: 'Generate' })
    await user.click(generate)
    await waitFor(() => expect(posted).toHaveLength(1))

    const { idempotency_key: firstKey, ...body } = posted[0]!
    expect(body).toEqual({
      prompt: 'Warm jazz piano',
      title: 'Rainy night',
      project_id: null,
      genre_id: 'g-jazz',
      subgenre_id: 'g-bebop',
      preset_id: null,
      model_id: 'm-a',
      lora_id: null,
      lora_scale: null,
      duration: 90,
      bpm: 80,
      key: 'A minor',
      time_signature: null,
      seed: 1234,
      instruments: ['piano'],
      moods: ['late night'],
      instrumental: true,
      params: { inference_steps: null, guidance_scale: 7.5 },
      mode: 'text',
      reference_audio_id: null,
      source_audio_id: null,
    })
    expect(firstKey).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/)

    await waitFor(() => expect(generate).toBeEnabled())
    await user.click(generate)
    await waitFor(() => expect(posted).toHaveLength(2))
    expect(posted[1]!.idempotency_key).not.toBe(firstKey)
  })

  it('shows the API error message when the request is rejected', async () => {
    mockFetch({
      'GET /genres': genres,
      'GET /presets': presets,
      'GET /models': models,
      'GET /loras': [],
      'POST /generations': () =>
        jsonResponse({ error: { code: 'validation', message: 'Model đang bị tắt' } }, 422),
    })
    renderWithProviders(<GenerateForm />)
    const user = userEvent.setup()
    await user.type(await screen.findByLabelText('Prompt'), 'Dark ambient drone')
    await user.click(screen.getByRole('button', { name: 'Generate' }))
    expect(await screen.findByText('Model đang bị tắt')).toBeInTheDocument()
  })

  describe('optional Reference Audio', () => {
    // Model A can only do text; Model R also takes a Reference Audio.
    const withAudioModel = [
      models[0]!,
      makeModel({ id: 'm-r', name: 'Model R', capabilities: ['generate', 'lora', 'reference_audio', 'cover'] }),
    ]
    const mp3 = () => new File(['ID3 fake audio'], 'ref-song.mp3', { type: 'audio/mpeg' })

    it('is one form: no mode switch, an optional Reference Audio, no Cover / Remix', async () => {
      setup()
      expect(await screen.findByLabelText('Chọn file Reference Audio')).toBeInTheDocument()
      expect(screen.getByText(/Không bắt buộc/)).toBeInTheDocument()
      expect(screen.queryByRole('radiogroup')).not.toBeInTheDocument()
      expect(screen.queryByText(/Cover \/ Remix/)).not.toBeInTheDocument()
      expect(screen.queryByText(/Source Audio/)).not.toBeInTheDocument()
      expect(screen.getByLabelText('Duration')).toBeInTheDocument()
    })

    it('with a file: uploads it, previews it and sends a reference job; removing it sends a text job', async () => {
      const { user, posted, uploads } = setup([], withAudioModel)
      await user.selectOptions(await screen.findByLabelText('Model'), 'm-r')
      await user.type(screen.getByLabelText('Prompt'), 'Dark ambient drone')

      await user.upload(screen.getByLabelText('Chọn file Reference Audio'), mp3())
      expect(await screen.findByText('ref-song.mp3')).toBeInTheDocument()
      expect(uploads.map((f) => f.name)).toEqual(['ref-song.mp3'])
      expect(screen.getByLabelText('Nghe thử Reference Audio')).toHaveAttribute(
        'src',
        '/api/v1/artifacts/in-1/content',
      )

      await user.click(screen.getByRole('button', { name: 'Generate' }))
      await waitFor(() => expect(posted).toHaveLength(1))
      expect(posted[0]).toMatchObject({
        mode: 'reference',
        reference_audio_id: 'in-1',
        source_audio_id: null,
        duration: 120,
        params: { inference_steps: null, guidance_scale: null },
      })
      expect(posted[0]!.params).not.toHaveProperty('audio_cover_strength')
    })

    it('blocks a model that cannot use the attached file instead of dropping it silently', async () => {
      const { user } = setup([], withAudioModel)
      const model = await screen.findByLabelText('Model')
      await user.selectOptions(model, 'm-r')
      await user.upload(screen.getByLabelText('Chọn file Reference Audio'), mp3())
      await screen.findByText('ref-song.mp3')
      expect(screen.getByRole('button', { name: 'Generate' })).toBeEnabled()

      await user.selectOptions(model, 'm-a')
      expect(screen.getByText(/không dùng được Reference Audio/)).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Generate' })).toBeDisabled()

      // Removing the file brings the dropzone back and unblocks the submit.
      await user.click(screen.getByRole('button', { name: 'Bỏ file Reference Audio' }))
      expect(screen.getByLabelText('Chọn file Reference Audio')).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Generate' })).toBeEnabled()
    })
  })

  describe('prompt written from the Reference Audio', () => {
    const mp3 = () => new File(['ID3 fake audio'], 'ref-song.mp3', { type: 'audio/mpeg' })
    const CAPTION = 'A chill lo-fi track with warm Rhodes.'
    const done = (result: Record<string, unknown>) =>
      makeJob({ id: 'pj-1', type: 'describe_input', status: 'completed', result })
    const routes = (job: unknown, calls: string[] = []) => ({
      'POST /audio-inputs/in-1/prompt-jobs': () => {
        calls.push('post')
        return jsonResponse({ job_id: 'pj-1', model_key: 'acestep-captioner' }, 201)
      },
      'GET /jobs/pj-1': job,
    })

    it('fills an empty prompt, offers the measured tempo/key and lets the user edit it', async () => {
      const { user } = setup(baseLoras, models, 'member', presets, undefined,
        routes(done({ caption: CAPTION, tempo_bpm: 82.4, key: 'A minor' })))
      await user.upload(await screen.findByLabelText('Chọn file Reference Audio'), mp3())
      const prompt = screen.getByLabelText('Prompt')
      await waitFor(() => expect(prompt).toHaveValue(CAPTION))
      expect(screen.getByText(/Tự tạo từ ref-song.mp3/)).toBeInTheDocument()
      await user.click(screen.getByRole('button', { name: 'Dùng Tempo/Key này' }))
      expect(screen.getByLabelText('BPM')).toHaveValue(82)
      expect(screen.getByLabelText('Key')).toHaveValue('A minor')
      await user.type(prompt, ' Slower.')
      expect(prompt).toHaveValue(`${CAPTION} Slower.`)
    })

    it('never overwrites a typed prompt: offers it as a suggestion instead', async () => {
      const { user } = setup(baseLoras, models, 'member', presets, undefined, routes(done({ caption: CAPTION })))
      const prompt = await screen.findByLabelText('Prompt')
      await user.type(prompt, 'rainy night')
      await user.upload(screen.getByLabelText('Chọn file Reference Audio'), mp3())
      expect(await screen.findByText(/Gợi ý từ ref-song.mp3/)).toBeInTheDocument()
      expect(prompt).toHaveValue('rainy night')
      await user.click(screen.getByRole('button', { name: 'Thêm vào cuối' }))
      expect(prompt).toHaveValue(`rainy night, ${CAPTION}`)
      expect(screen.queryByText(/Gợi ý từ ref-song.mp3/)).toBeNull()
    })

    it('"Viết lại" asks again and "Hoàn tác" brings the earlier prompt back', async () => {
      const calls: string[] = []
      const { user } = setup(baseLoras, models, 'member', presets, undefined, routes(done({ caption: CAPTION }), calls))
      await user.upload(await screen.findByLabelText('Chọn file Reference Audio'), mp3())
      const prompt = screen.getByLabelText('Prompt')
      await waitFor(() => expect(prompt).toHaveValue(CAPTION))
      await user.clear(prompt)
      await user.type(prompt, 'my edit')
      await user.click(screen.getByRole('button', { name: 'Viết lại' }))
      await waitFor(() => expect(prompt).toHaveValue(CAPTION))
      expect(calls).toEqual(['post', 'post'])
      await user.click(screen.getByRole('button', { name: 'Hoàn tác' }))
      expect(prompt).toHaveValue('my edit')
    })

    it('a failed job says so and can be retried, the form still works', async () => {
      const { user } = setup(baseLoras, models, 'member', presets, undefined,
        routes(makeJob({ id: 'pj-1', type: 'describe_input', status: 'failed', error: 'boom' })))
      await user.upload(await screen.findByLabelText('Chọn file Reference Audio'), mp3())
      expect(await screen.findByText(/Không viết được prompt/)).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Thử lại' })).toBeInTheDocument()
      expect(screen.getByLabelText('Prompt')).toHaveValue('')
    })

    it('counts the composed prompt against what ACE-Step reads', async () => {
      const { user } = setup()
      const prompt = await screen.findByLabelText('Prompt')
      expect(screen.getByText('0/850')).toBeInTheDocument()
      await user.type(prompt, 'rain')
      // "rain, instrumental, no vocals"
      expect(screen.getByText('29/850')).toBeInTheDocument()
      await user.click(prompt)
      await user.paste('x'.repeat(900))
      expect(screen.getByText(/ACE-Step chỉ đọc khoảng 850 ký tự/)).toBeInTheDocument()
    })
  })
})
