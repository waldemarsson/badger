<script lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { onMount } from 'svelte';
import { flip } from 'svelte/animate';
import { fade } from 'svelte/transition';
import type { TodoRow, TodoStatus } from '#lib/types.js';
import { COLUMNS, columnCards, dueState, formatDue } from '#lib/todos.js';
import TodoModal from '#lib/components/TodoModal.svelte';
import { changes } from '#lib/palette-state.svelte.js';
import Plus from '@lucide/svelte/icons/plus';
import X from '@lucide/svelte/icons/x';
import CalendarDays from '@lucide/svelte/icons/calendar-days';
import Link2 from '@lucide/svelte/icons/link-2';

const DRAG_THRESHOLD = 5; // px of movement before a press becomes a drag

let todos = $state<TodoRow[]>([]);
let error = $state('');
let selected = $state<TodoRow | null>(null);

// Add card
let addingTo = $state<TodoStatus | null>(null);
let addTitle = $state('');

// Drag state. `pending` = pressed but not yet moved past the threshold.
type Press = { todo: TodoRow; startX: number; startY: number; offX: number; offY: number; width: number };
let pending: Press | null = null;
let drag = $state<(Press & { x: number; y: number; status: TodoStatus; index: number }) | null>(null);

async function load(): Promise<void> {
try {
todos = await invoke<TodoRow[]>('list_todos');
} catch (e) {
error = String(e);
}
}

// ── Board rendering ───────────────────────────────────────────────────────────

type Slot = { key: number | 'placeholder'; todo?: TodoRow };

function slots(status: TodoStatus): Slot[] {
const items: Slot[] = columnCards(todos, status, drag?.todo.id).map((t) => ({ key: t.id, todo: t }));
if (drag && drag.status === status) items.splice(drag.index, 0, { key: 'placeholder' });
return items;
}

function count(status: TodoStatus): number {
return todos.filter((t) => t.status === status).length;
}

const dueClasses: Record<string, string> = {
overdue: 'bg-red-50 dark:bg-red-500/10 text-red-700 dark:text-red-300 ring-red-200 dark:ring-red-500/30',
today: 'bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 ring-amber-200 dark:ring-amber-500/30',
upcoming: 'bg-slate-50 dark:bg-neutral-800/40 text-slate-600 dark:text-neutral-400 ring-slate-200 dark:ring-neutral-800'
};

// ── Drag and drop (pointer events: full control over the drag visuals and no
//    interference from the WebView's native file-drop handling) ─────────────────

function onCardPointerDown(e: PointerEvent, todo: TodoRow): void {
if (e.button !== 0) return;
e.preventDefault(); // no text selection while dragging
const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
pending = {
todo,
startX: e.clientX,
startY: e.clientY,
offX: e.clientX - rect.left,
offY: e.clientY - rect.top,
width: rect.width
};
window.addEventListener('pointermove', onPointerMove);
window.addEventListener('pointerup', onPointerUp);
window.addEventListener('pointercancel', cancelDrag);
}

function onPointerMove(e: PointerEvent): void {
if (!drag && pending) {
if (Math.hypot(e.clientX - pending.startX, e.clientY - pending.startY) < DRAG_THRESHOLD) return;
const index = columnCards(todos, pending.todo.status).findIndex((t) => t.id === pending!.todo.id);
drag = { ...pending, x: e.clientX, y: e.clientY, status: pending.todo.status, index };
document.body.classList.add('is-dragging');
}
if (!drag) return;
drag.x = e.clientX;
drag.y = e.clientY;
updateDropTarget(e.clientX, e.clientY);
}

/** Column under the pointer, and the slot whose vertical midpoint the pointer is above. */
function updateDropTarget(x: number, y: number): void {
if (!drag) return;
const column = document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-column]');
if (!column) return; // outside every column: keep the last target
const cards = Array.from(column.querySelectorAll<HTMLElement>('[data-card]'));
let index = cards.findIndex((c) => {
const r = c.getBoundingClientRect();
return y < r.top + r.height / 2;
});
if (index === -1) index = cards.length;
const status = column.dataset.column as TodoStatus;
if (status !== drag.status || index !== drag.index) {
drag.status = status;
drag.index = index;
}
}

function stopListening(): void {
window.removeEventListener('pointermove', onPointerMove);
window.removeEventListener('pointerup', onPointerUp);
window.removeEventListener('pointercancel', cancelDrag);
document.body.classList.remove('is-dragging');
}

function cancelDrag(): void {
stopListening();
pending = null;
drag = null;
}

async function onPointerUp(): Promise<void> {
stopListening();
const press = pending;
const dropped = drag;
pending = null;
drag = null;

if (!dropped) {
if (press) selected = press.todo; // plain click → open the card
return;
}

const { todo, status, index } = dropped;
// Optimistic: reorder locally so the card lands instantly, then persist.
const column = columnCards(todos, status, todo.id);
column.splice(index, 0, { ...todo, status });
const positions = new Map(column.map((t, i) => [t.id, i]));
todos = todos.map((t) => (positions.has(t.id) ? { ...t, status, position: positions.get(t.id)! } : t));
try {
await invoke('move_todo', { id: todo.id, status, index });
} catch (e) {
error = String(e);
}
await load();
}

function onWindowKeydown(e: KeyboardEvent): void {
if (e.key === 'Escape' && drag) cancelDrag();
}

// ── Add card ──────────────────────────────────────────────────────────────────

function startAdd(status: TodoStatus): void {
addingTo = status;
addTitle = '';
}

async function addCard(): Promise<void> {
if (!addingTo || !addTitle.trim()) return;
try {
await invoke('create_todo', { title: addTitle, notes: null, dueDate: null, status: addingTo, issueId: null });
addTitle = '';
await load();
} catch (e) {
error = String(e);
}
}

function onAddKeydown(e: KeyboardEvent): void {
if (e.key === 'Enter' && !e.shiftKey) {
e.preventDefault();
addCard();
} else if (e.key === 'Escape') {
addingTo = null;
}
}

function autofocus(node: HTMLElement): void {
node.focus();
}

onMount(() => {
load();
return cancelDrag;
});

// Reload when the command palette adds a todo.
let seenTodoChanges = changes.todos;
$effect(() => {
if (changes.todos === seenTodoChanges) return;
seenTodoChanges = changes.todos;
void load();
});
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#snippet cardFace(t: TodoRow)}
<p class="text-sm font-medium leading-snug {t.status === 'done' ? 'text-slate-400 dark:text-neutral-500 line-through decoration-slate-300 dark:decoration-neutral-600' : 'text-slate-800 dark:text-neutral-200'}">
{t.title}
</p>
{#if t.notes}
<p class="mt-1 line-clamp-2 text-xs leading-relaxed text-slate-500 dark:text-neutral-400">{t.notes}</p>
{/if}
{#if t.due_date || t.issue_title}
<div class="mt-2.5 flex flex-wrap items-center gap-1.5">
{#if t.due_date}
<span class="kanban-badge {dueClasses[dueState(t)]}">
<CalendarDays size={12} />
{formatDue(t.due_date)}
</span>
{/if}
{#if t.issue_title}
<span class="kanban-badge max-w-full bg-violet-50 dark:bg-violet-500/10 text-violet-700 dark:text-violet-300 ring-violet-200 dark:ring-violet-500/30" title="{t.customer_name} / {t.project_name} / {t.issue_title}">
<Link2 size={12} class="shrink-0" />
<span class="truncate">{t.project_name} · {t.issue_title}</span>
</span>
{/if}
</div>
{/if}
{/snippet}

<div class="page-shell">
<div class="page-view">

<h1 class="sr-only">Todos</h1>

{#if error}
<div class="flex items-center justify-between rounded-xl bg-red-50 dark:bg-red-500/10 px-4 py-2 text-sm text-red-700 dark:text-red-300 ring-1 ring-red-200 dark:ring-red-500/30">
{error}
<button onclick={() => (error = '')} title="Dismiss"><X size={14} /></button>
</div>
{/if}

<!-- Bleeds to the window edges and fills the height below the nav (4rem + 1px border) and header (3.5rem). -->
<div class="-mx-3 !-mb-4 !mt-0 grid min-h-[calc(100vh-7.5rem-1px)] grid-cols-1 items-stretch divide-slate-200 sm:-mx-4 md:grid-cols-3 md:divide-x lg:-mx-6 dark:divide-neutral-800">
{#each COLUMNS as col (col.status)}
<section
data-column={col.status}
class="kanban-lane"
class:kanban-lane-target={drag?.status === col.status}
>
<header class="-mx-1 mb-3 flex items-center gap-2 border-b border-slate-200 px-1 py-3 dark:border-neutral-800">
<span class="h-2.5 w-2.5 rounded-full {col.dot}"></span>
<h2 class="text-xs font-semibold uppercase tracking-wider text-slate-600 dark:text-neutral-400">{col.label}</h2>
<span class="ml-auto rounded-full bg-white dark:bg-neutral-900 px-2 py-0.5 text-xs font-medium text-slate-500 dark:text-neutral-400 ring-1 ring-slate-200 dark:ring-neutral-800">
{count(col.status)}
</span>
</header>

<div class="flex min-h-[3rem] flex-col gap-2">
{#each slots(col.status) as slot (slot.key)}
<div animate:flip={{ duration: 200 }}>
{#if slot.todo}
{@const t = slot.todo}
<div
data-card
class="kanban-card"
class:opacity-75={t.status === 'done'}
role="button"
tabindex="0"
onpointerdown={(e) => onCardPointerDown(e, t)}
onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), (selected = t))}
>
{@render cardFace(t)}
</div>
{:else if drag}
<div class="kanban-placeholder" in:fade={{ duration: 120 }}>
<div class="invisible">{@render cardFace(drag.todo)}</div>
</div>
{/if}
</div>
{:else}
{#if !drag}
<p class="py-3 text-center text-xs text-slate-400 dark:text-neutral-500">No cards</p>
{/if}
{/each}
</div>

{#if addingTo === col.status}
<div class="mt-2 rounded-xl bg-white dark:bg-neutral-800 p-2 shadow-sm ring-1 ring-slate-200 dark:ring-neutral-700" in:fade={{ duration: 120 }}>
<textarea
use:autofocus
class="w-full resize-none rounded-lg border-0 p-1 text-sm text-slate-800 dark:text-neutral-200 outline-none placeholder:text-slate-400 dark:placeholder:text-neutral-500 dark:bg-transparent focus:ring-0"
rows="2"
placeholder="What needs doing?"
bind:value={addTitle}
onkeydown={onAddKeydown}
></textarea>
<div class="flex items-center gap-2 pt-1">
<button class="btn-secondary-sm" onclick={addCard}>Add card</button>
<button class="rounded-md p-1 text-slate-400 dark:text-neutral-500 transition hover:bg-slate-100 dark:hover:bg-neutral-800 hover:text-slate-700 dark:hover:text-neutral-300" onclick={() => (addingTo = null)} title="Close (Esc)">
<X size={16} />
</button>
</div>
</div>
{:else}
<button class="kanban-add-card" onclick={() => startAdd(col.status)}>
<Plus size={15} /> Add card
</button>
{/if}
</section>
{/each}
</div>

</div>
</div>

{#if drag}
<!-- Floating copy of the dragged card; follows the pointer. -->
<div
class="pointer-events-none fixed left-0 top-0 z-50"
style="width: {drag.width}px; transform: translate({drag.x - drag.offX}px, {drag.y - drag.offY}px);"
>
<div class="kanban-card kanban-drag-lift">
{@render cardFace(drag.todo)}
</div>
</div>
{/if}

{#if selected}
<TodoModal todo={selected} onclose={() => (selected = null)} onchange={load} />
{/if}
