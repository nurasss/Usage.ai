export function formatCompact(value: number): string {
  return new Intl.NumberFormat('ru-RU', { notation: 'compact', maximumFractionDigits: 1 }).format(value);
}

export function formatAge(iso: string, now = Date.now()): string {
  const seconds = Math.max(0, Math.floor((now - new Date(iso).getTime()) / 1000));
  if (seconds < 60) return 'только что';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} мин назад`;
  return `${Math.floor(seconds / 3600)} ч назад`;
}

export function formatCountdown(iso: string, now = Date.now()): string {
  const ms = new Date(iso).getTime() - now;
  if (ms <= 0) return 'Ожидается подтверждение';
  const mins = Math.ceil(ms / 60_000);
  if (mins < 60) return `Сброс через ${mins} мин`;
  if (mins < 24 * 60) return `Сброс через ${Math.floor(mins / 60)} ч ${mins % 60} мин`;
  const date = new Date(iso);
  const tomorrow = new Date(now); tomorrow.setDate(tomorrow.getDate() + 1);
  if (date.toDateString() === tomorrow.toDateString()) return `Сброс завтра в ${date.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })}`;
  return `Сброс ${date.toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' })} в ${date.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit' })}`;
}

const labels: Record<string, string> = {
  Connected: 'Подключено', Refreshing: 'Обновление', NotConfigured: 'Не настроено',
  AuthenticationRequired: 'Нужна авторизация', SessionExpired: 'Сессия истекла',
  PermissionDenied: 'Нет разрешения', InsufficientScope: 'Недостаточно прав',
  ApplicationNotRunning: 'Приложение не запущено', RateLimited: 'Ограничено источником',
  Cooldown: 'Пауза до следующей попытки', Unavailable: 'Недоступно',
  UnsupportedSource: 'Источник не поддерживается', ParseError: 'Формат источника изменился',
  NetworkError: 'Ошибка сети', Offline: 'Офлайн', UnknownError: 'Неизвестная ошибка'
};

export function stateLabel(state: string): string { return labels[state] ?? state; }
