import { describe, expect, it } from 'vitest';
import { formatCountdown } from './format';

describe('formatCountdown', () => {
  const now = new Date('2026-09-08T12:00:00Z').getTime();
  it('shows minutes', () => expect(formatCountdown('2026-09-08T12:42:00Z', now)).toBe('Сброс через 42 мин'));
  it('does not claim a reset after expiry', () => expect(formatCountdown('2026-09-08T11:59:00Z', now)).toBe('Ожидается подтверждение'));
});

