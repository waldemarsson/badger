<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Task } from '#lib/types.js';
	import { completedLabel, needsAttention, taskChips } from '#lib/tasks.js';
	import TaskCheck from '#lib/components/TaskCheck.svelte';
	import TaskChips from '#lib/components/TaskChips.svelte';

	// One task: the check, then title, a notes line and chips (open), or when it was
	// completed (done). Used by /tasks and the Today card.
	let {
		task,
		now,
		selected = false,
		ticking = false,
		syncOn = false,
		onselect,
		ontoggle,
		actions
	}: {
		task: Task;
		now: Date;
		selected?: boolean;
		/** Mid-completion: the check pops and the title strikes through. */
		ticking?: boolean;
		/** Apple Reminders sync is on: show "Mac only". */
		syncOn?: boolean;
		onselect: () => void;
		ontoggle: () => void;
		/** Row buttons after the title (e.g. Today's Log time), given the task. */
		actions?: Snippet<[Task]>;
	} = $props();

	const chips = $derived(task.done ? [] : taskChips(task, now, syncOn));
</script>

<div class="task-row" class:task-row-selected={selected} data-task-row={task.id}>
	<TaskCheck
		done={task.done}
		attention={needsAttention(task, now)}
		{ticking}
		label={task.done ? `Mark ${task.title} as not done` : `Complete ${task.title}`}
		onclick={ontoggle}
	/>
	<button type="button" class="task-row-button" aria-current={selected ? 'true' : undefined} onclick={onselect}>
		<span class="task-title" class:task-title-done={task.done && !ticking} class:task-title-striking={ticking} title={task.title.length > 80 ? task.title : undefined}>{task.title}</span>
		{#if task.done}
			<span class="task-notes">{completedLabel(task, now)}</span>
		{:else}
			{#if task.notes}
				<span class="task-notes">{task.notes}</span>
			{/if}
			{#if chips.length}
				<span class="task-chips"><TaskChips {chips} /></span>
			{/if}
		{/if}
	</button>
	{#if actions}
		<div class="-my-1 flex shrink-0 items-center gap-1">{@render actions(task)}</div>
	{/if}
</div>
