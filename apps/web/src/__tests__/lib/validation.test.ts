import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import { validateForm } from '@/lib/validation';

const schema = z.object({
  username: z.string().trim().min(1, 'Username is required'),
  email: z.string().email('Enter a valid email'),
});

describe('validateForm', () => {
  it('returns the parsed data when valid', () => {
    const result = validateForm(schema, { username: 'ana', email: 'ana@feuer.me' });

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.data.username).toBe('ana');
    }
  });

  it('maps each invalid field to its first message', () => {
    const result = validateForm(schema, { username: '  ', email: 'nope' });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.username).toBe('Username is required');
      expect(result.errors.email).toBe('Enter a valid email');
    }
  });

  it('keeps only the first error per field', () => {
    const pw = z.object({
      password: z.string().min(8, 'Too short').regex(/\d/, 'Needs a digit'),
    });

    const result = validateForm(pw, { password: 'ab' });

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.password).toBe('Too short');
    }
  });
});
