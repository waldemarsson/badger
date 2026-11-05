import { today } from '$lib/date-ranges';
import type { TodoRow, TodoStatus } from '$lib/types';

export const COLUMNS: { status: TodoStatus; label: string; dot: string }[] = [
	{ status: 'not_started', label: 'Not started', dot: 'bg-slate-400' },
	{ status: 'in_progress', label: 'In progress', dot: 'bg-sky-500' },
	{ status: 'done', label: 'Done', dot: 'bg-emerald-500' }
];

/** Cards of one column in board order. */
export function columnCards(todos: TodoRow[], status: TodoStatus, excludeId?: number): TodoRow[] {
	return todos
		.filter((t) => t.status === status && t.id !== excludeId)
		.sort((a, b) => a.position - b.position || a.id - b.id);
}

export type DueState = 'overdue' | 'today' | 'upcoming' | 'none';

export function dueState(t: TodoRow): DueState {
	if (!t.due_date || t.status === 'done') return t.due_date ? 'upcoming' : 'none';
	const now = today();
	if (t.due_date < now) return 'overdue';
	if (t.due_date === now) return 'today';
	return 'upcoming';
}

/** "Nov 5" for YYYY-MM-DD, parsed as a local date. */
export function formatDue(date: string): string {
	const [y, m, d] = date.split('-').map(Number);
	return new Date(y, m - 1, d).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
