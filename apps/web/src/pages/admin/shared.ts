import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useToast } from '../../components/Toast'

/**
 * Mutation for admin screens: on success refreshes the given query-key prefix and
 * optionally toasts; on failure toasts the API's error message.
 */
export function useAdminMutation<TInput, TResult>(
  mutationFn: (input: TInput) => Promise<TResult>,
  options: { invalidate: readonly unknown[]; successMessage?: string; onSuccess?: (result: TResult) => void },
) {
  const queryClient = useQueryClient()
  const toast = useToast()
  return useMutation({
    mutationFn,
    onSuccess: (result) => {
      void queryClient.invalidateQueries({ queryKey: options.invalidate })
      if (options.successMessage) toast.success(options.successMessage)
      options.onSuccess?.(result)
    },
    onError: (error) => toast.error(error),
  })
}

/** "Nhạc Lo-fi Đêm" -> "nhac-lo-fi-dem" */
export function slugify(text: string): string {
  return text
    .normalize('NFD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/đ/g, 'd')
    .replace(/Đ/g, 'D')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

export function numberOrNull(text: string): number | null {
  const trimmed = text.trim()
  if (trimmed === '') return null
  const n = Number(trimmed)
  return Number.isFinite(n) ? n : null
}

export function numberToText(value: unknown): string {
  return typeof value === 'number' && Number.isFinite(value) ? String(value) : ''
}
