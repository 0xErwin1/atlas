import type { ZodType } from 'zod';

export type FieldErrors<T> = Partial<Record<keyof T, string>>;

export type ValidationResult<T> = { ok: true; data: T } | { ok: false; errors: FieldErrors<T> };

/**
 * Validate `values` against a zod `schema`, returning either the parsed data or
 * a flat map of field → first error message. Only the first issue per field is
 * kept so each form field shows a single inline message; nested paths collapse
 * to their top-level field key.
 */
export function validateForm<T>(schema: ZodType<T>, values: unknown): ValidationResult<T> {
  const result = schema.safeParse(values);

  if (result.success) {
    return { ok: true, data: result.data };
  }

  const errors: FieldErrors<T> = {};
  for (const issue of result.error.issues) {
    const key = issue.path[0];
    if (typeof key === 'string' && !(key in errors)) {
      errors[key as keyof T] = issue.message;
    }
  }

  return { ok: false, errors };
}
