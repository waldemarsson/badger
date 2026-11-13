<script lang="ts">
	// Settings › Backup & Restore: where automatic backups go, whether they are encrypted,
	// how many to keep, and the backups currently in that folder.
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
	import { invoke } from '@tauri-apps/api/core';
	import { open } from '@tauri-apps/plugin-dialog';
	import Check from '@lucide/svelte/icons/check';
	import Copy from '@lucide/svelte/icons/copy';
	import FolderOpen from '@lucide/svelte/icons/folder-open';
	import HardDriveUpload from '@lucide/svelte/icons/hard-drive-upload';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import Lock from '@lucide/svelte/icons/lock';
	import LockOpen from '@lucide/svelte/icons/lock-open';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import { DECRYPT_COMMAND, lastBackupLabel, shortKey } from '#lib/backup.js';
	import type { BackupSettings, BackupStatus, GeneratedBackupKey, StoredBackup } from '#lib/types.js';

	let { formatDate, formatSize }: {
		formatDate: (date: string) => string;
		formatSize: (bytes: number) => string;
	} = $props();

	let settings = $state<BackupSettings | null>(null);
	let status = $state<BackupStatus>({ last_success: null, last_error: null });
	let backups = $state<StoredBackup[]>([]);
	let backupsState = $state<'loading' | 'ready' | 'error'>('loading');
	let backupsError = $state('');
	let loadError = $state('');

	let running = $state(false);
	let folderError = $state('');

	// Encryption setup: null when idle, else the flow in progress.
	let setup = $state<null | 'generated' | 'own'>(null);
	let generated = $state<GeneratedBackupKey | null>(null);
	let confirmSecret = $state('');
	let ownPublicKey = $state('');
	let ownConfirmed = $state(false);
	let copied = $state(false);
	let keyError = $state('');
	let saving = $state(false);
	let confirmingDisable = $state(false);
	let confirmingDeletePlain = $state(false);
	let deletePlainError = $state('');

	let retention = $state('');
	let retentionState = $state<'loading' | 'idle' | 'saving' | 'success' | 'error'>('loading');
	let retentionError = $state('');

	const encrypted = $derived(settings?.public_key != null);
	const plainCount = $derived(backups.filter((b) => !b.encrypted).length);

	onMount(() => {
		void load();
	});

	async function load() {
		loadError = '';
		try {
			const [s, r] = await Promise.all([
				invoke<BackupSettings>('get_backup_settings'),
				invoke<number>('get_startup_backup_retention'),
			]);
			settings = s;
			retention = String(r);
			retentionState = 'idle';
		} catch (e) {
			loadError = String(e);
			retentionState = 'error';
		}
		await Promise.all([loadBackups(), loadStatus()]);
	}

	async function loadBackups() {
		try {
			backups = await invoke<StoredBackup[]>('list_startup_backups');
			backupsState = 'ready';
		} catch (e) {
			backups = [];
			backupsError = String(e);
			backupsState = 'error';
		}
	}

	async function loadStatus() {
		try {
			status = await invoke<BackupStatus>('get_backup_status');
		} catch {
			// The status line is informational; a failure leaves the previous one.
		}
	}

	async function backUpNow() {
		running = true;
		try {
			await invoke('back_up_now');
		} catch {
			// Recorded in the backup status, shown below.
		} finally {
			running = false;
			await Promise.all([loadBackups(), loadStatus()]);
		}
	}

	/** Saves a settings change, then writes a backup right away to prove the new setup works. */
	async function applySettings(command: string, args: Record<string, unknown>) {
		settings = await invoke<BackupSettings>(command, args);
		await backUpNow();
	}

	async function chooseFolder() {
		folderError = '';
		const dir = await open({ directory: true, defaultPath: settings?.dir });
		if (typeof dir !== 'string') return;
		try {
			await applySettings('set_backup_dir', { dir });
		} catch (e) {
			folderError = String(e);
		}
	}

	async function useDefaultFolder() {
		folderError = '';
		try {
			await applySettings('set_backup_dir', { dir: null });
		} catch (e) {
			folderError = String(e);
		}
	}

	function resetSetup() {
		setup = null;
		generated = null;
		confirmSecret = '';
		ownPublicKey = '';
		ownConfirmed = false;
		copied = false;
		keyError = '';
	}

	async function startGenerate() {
		resetSetup();
		confirmingDisable = false;
		try {
			generated = await invoke<GeneratedBackupKey>('generate_backup_key');
			setup = 'generated';
		} catch (e) {
			keyError = String(e);
		}
	}

	function startOwn() {
		resetSetup();
		confirmingDisable = false;
		setup = 'own';
	}

	async function copySecret() {
		if (!generated) return;
		try {
			await navigator.clipboard.writeText(generated.secret_key);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			keyError = 'Could not copy. Select the key and copy it by hand.';
		}
	}

	async function enable() {
		keyError = '';
		const publicKey = setup === 'generated' ? generated?.public_key : ownPublicKey;
		if (!publicKey?.trim()) {
			keyError = 'Enter your age public key.';
			return;
		}
		if (setup === 'generated' && !confirmSecret.trim()) {
			keyError = 'Paste the secret key you saved to confirm it.';
			return;
		}
		if (setup === 'own' && !ownConfirmed) {
			keyError = 'Confirm that you have the matching secret key.';
			return;
		}
		saving = true;
		try {
			await applySettings('enable_backup_encryption', {
				publicKey,
				secretKey: setup === 'generated' ? confirmSecret : null,
			});
			resetSetup();
		} catch (e) {
			keyError = String(e);
		} finally {
			saving = false;
		}
	}

	async function disable() {
		keyError = '';
		try {
			await applySettings('disable_backup_encryption', {});
			confirmingDisable = false;
		} catch (e) {
			keyError = String(e);
		}
	}

	async function deletePlain() {
		deletePlainError = '';
		try {
			await invoke<number>('delete_unencrypted_backups');
			confirmingDeletePlain = false;
		} catch (e) {
			deletePlainError = String(e);
		}
		await loadBackups();
	}

	async function saveRetention() {
		retentionError = '';
		const value = retention.trim();
		if (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value))) {
			retentionState = 'error';
			retentionError = 'Enter a whole number, 0 or more.';
			return;
		}
		retentionState = 'saving';
		try {
			retention = String(await invoke<number>('set_startup_backup_retention', { value: Number(value) }));
			retentionState = 'success';
		} catch (e) {
			retentionState = 'error';
			retentionError = String(e);
		}
	}

	function onSetupKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') void enable();
		if (e.key === 'Escape') resetSetup();
	}
</script>

<section class="panel-card space-y-5">
	<div class="flex flex-wrap items-start justify-between gap-3">
		<div>
			<h2 class="panel-title">Automatic Backups</h2>
			<p class="panel-copy">A backup is written on startup and again each new day while Badger stays open.</p>
		</div>
		<div class="flex items-center gap-3">
			<span class="text-sm text-slate-500 dark:text-neutral-400" aria-live="polite">
				{#if running}
					Backing up…
				{:else if status.last_success}
					Last backup {lastBackupLabel(status.last_success, new Date())}
				{/if}
			</span>
			<button class="btn-secondary" onclick={backUpNow} disabled={running || !settings}>
				<HardDriveUpload size={16} class="mr-2" />
				Back up now
			</button>
		</div>
	</div>

	{#if loadError}
		<div class="stack-error">{loadError}</div>
	{/if}

	{#if status.last_error && !running}
		<div class="alert-panel alert-error" transition:fade={{ duration: 150 }}>
			<p class="alert-error-title">The last backup failed</p>
			<p class="alert-error-copy">{status.last_error}</p>
			<p class="alert-error-copy mt-1">Badger tries again every 10 minutes. Nothing is written anywhere else.</p>
		</div>
	{/if}

	{#if settings}
		<!-- Folder -->
		<div class="panel-muted space-y-2">
			<div class="flex flex-wrap items-center justify-between gap-3">
				<div class="min-w-0">
					<p class="form-label">Folder</p>
					<p class="truncate font-mono text-sm text-slate-900 dark:text-neutral-100" title={settings.dir}>{settings.dir}</p>
				</div>
				<div class="flex shrink-0 gap-2">
					{#if !settings.is_default_dir}
						<button class="btn-secondary-sm" onclick={useDefaultFolder}>
							<RotateCcw size={14} class="mr-1.5" />
							Use default
						</button>
					{/if}
					<button class="btn-secondary-sm" onclick={chooseFolder}>
						<FolderOpen size={14} class="mr-1.5" />
						Change…
					</button>
				</div>
			</div>
			<p class="text-xs text-slate-500 dark:text-neutral-400">
				Pick a synced folder (iCloud Drive, Dropbox) to keep copies off this Mac. Existing backups are not moved.
			</p>
			{#if folderError}
				<p class="form-error" role="alert">{folderError}</p>
			{/if}
		</div>

		<!-- Encryption -->
		<div class="panel-muted space-y-3">
			<div class="flex flex-wrap items-center justify-between gap-3">
				<div class="min-w-0">
					<p class="form-label">Encryption</p>
					{#if encrypted && settings.public_key}
						<p class="flex items-center gap-1.5 text-sm text-emerald-700 dark:text-emerald-400">
							<Lock size={14} />
							On — encrypted to <span class="font-mono" title={settings.public_key}>{shortKey(settings.public_key)}</span>
						</p>
					{:else}
						<p class="flex items-center gap-1.5 text-sm text-slate-600 dark:text-neutral-400">
							<LockOpen size={14} />
							Off — backups are readable by anyone with access to the folder.
						</p>
					{/if}
				</div>
				{#if !setup}
					<div class="flex shrink-0 gap-2">
						{#if encrypted}
							{#if confirmingDisable}
								<span class="self-center text-sm text-slate-600 dark:text-neutral-400">Write unencrypted backups?</span>
								<button class="btn-danger-sm" onclick={disable}>Turn off</button>
								<button class="btn-secondary-sm" onclick={() => (confirmingDisable = false)}>Cancel</button>
							{:else}
								<button class="btn-secondary-sm" onclick={startGenerate}>Change key</button>
								<button class="btn-secondary-sm" onclick={() => (confirmingDisable = true)}>Turn off</button>
							{/if}
						{:else}
							<button class="btn-secondary-sm" onclick={startOwn}>Use my own key</button>
							<button class="btn-primary" onclick={startGenerate}>
								<KeyRound size={16} class="mr-2" />
								Set up encryption
							</button>
						{/if}
					</div>
				{/if}
			</div>

			{#if setup}
				<div class="space-y-3 border-t border-slate-200 pt-3 dark:border-neutral-800" transition:fade={{ duration: 150 }}>
					{#if encrypted}
						<div class="alert-panel alert-warning">
							<p class="alert-warning-copy">Backups already written stay encrypted to the old key. Keep it until they have aged out.</p>
						</div>
					{/if}

					{#if setup === 'generated' && generated}
						<div class="alert-panel alert-warning">
							<p class="alert-warning-title">Save this secret key in your password manager now</p>
							<p class="alert-warning-copy">Badger never stores it and won't show it again. Without it, encrypted backups can't be restored by anyone.</p>
						</div>
						<div>
							<p class="form-label mb-1">Secret key</p>
							<div class="flex items-center gap-2">
								<code class="block min-w-0 flex-1 select-all break-all rounded-xl bg-white px-3 py-2 font-mono text-xs text-slate-900 ring-1 ring-slate-200 dark:bg-neutral-950 dark:text-neutral-100 dark:ring-neutral-800">{generated.secret_key}</code>
								<button class="btn-secondary-sm shrink-0" onclick={copySecret}>
									{#if copied}
										<Check size={14} class="mr-1.5 text-emerald-600 dark:text-emerald-400" />Copied
									{:else}
										<Copy size={14} class="mr-1.5" />Copy
									{/if}
								</button>
							</div>
						</div>
						<div>
							<label class="form-label mb-1" for="backup-confirm-secret">Paste the secret key to confirm you saved it</label>
							<input
								id="backup-confirm-secret"
								class="form-input font-mono"
								type="password"
								autocomplete="off"
								spellcheck="false"
								placeholder="AGE-SECRET-KEY-1…"
								bind:value={confirmSecret}
								onkeydown={onSetupKeydown}
							/>
						</div>
					{:else if setup === 'own'}
						<div>
							<label class="form-label mb-1" for="backup-own-key">Your age public key</label>
							<!-- svelte-ignore a11y_autofocus -->
							<input
								id="backup-own-key"
								class="form-input font-mono"
								type="text"
								autocomplete="off"
								spellcheck="false"
								placeholder="age1…"
								autofocus
								bind:value={ownPublicKey}
								onkeydown={onSetupKeydown}
							/>
						</div>
						<label class="flex items-center gap-2 text-sm text-slate-700 dark:text-neutral-300">
							<input type="checkbox" bind:checked={ownConfirmed} />
							I have the matching secret key saved somewhere safe
						</label>
					{/if}

					{#if keyError}
						<p class="form-error" role="alert">{keyError}</p>
					{/if}

					<div class="flex gap-2">
						<button class="btn-primary" onclick={enable} disabled={saving}>
							<Lock size={16} class="mr-2" />
							{saving ? 'Turning on…' : 'Turn on encryption'}
						</button>
						<button class="btn-secondary" onclick={resetSetup} disabled={saving}>Cancel</button>
					</div>
				</div>
			{:else if keyError}
				<p class="form-error" role="alert">{keyError}</p>
			{/if}

			{#if encrypted && !setup}
				<p class="text-xs text-slate-500 dark:text-neutral-400">
					Restore in Badger with the secret key, or without Badger: <code class="font-mono">{DECRYPT_COMMAND}</code>
				</p>
			{/if}
		</div>

		<!-- Retention + stored backups -->
		<div class="grid gap-4 lg:grid-cols-[minmax(0,14rem)_1fr]">
			<div class="space-y-2">
				<label class="form-label" for="backup-retention">Keep (days)</label>
				<div class="flex gap-2">
					<input
						id="backup-retention"
						class="form-input"
						type="text"
						inputmode="numeric"
						autocomplete="off"
						bind:value={retention}
						oninput={() => { retentionError = ''; if (retentionState !== 'loading') retentionState = 'idle'; }}
						onkeydown={(e) => { if (e.key === 'Enter') void saveRetention(); }}
						disabled={retentionState === 'loading' || retentionState === 'saving'}
					/>
					<button class="btn-secondary" onclick={saveRetention} disabled={retentionState === 'loading' || retentionState === 'saving'}>
						{#if retentionState === 'success'}<Check size={16} class="text-emerald-600 dark:text-emerald-400" />{:else}Save{/if}
					</button>
				</div>
				<p class="text-xs text-slate-500 dark:text-neutral-400">0 turns automatic backups off.</p>
				{#if retentionError}
					<p class="form-error" role="alert">{retentionError}</p>
				{/if}
			</div>

			<div class="space-y-3">
				{#if backupsState === 'loading'}
					<p class="text-sm text-slate-500 dark:text-neutral-400">Loading backups…</p>
				{:else if backupsState === 'error'}
					<div class="stack-error">{backupsError}</div>
				{:else if backups.length === 0}
					<div class="empty-state">
						<p class="font-semibold text-slate-700 dark:text-neutral-300">No backups in this folder yet</p>
						<p>Backups appear here after the next one is written.</p>
					</div>
				{:else}
					<div class="overflow-hidden rounded-2xl border border-slate-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
						<table class="table-base">
							<thead class="table-head">
								<tr>
									<th class="table-heading">Date</th>
									<th class="table-heading">Type</th>
									<th class="table-heading text-right">Size</th>
								</tr>
							</thead>
							<tbody>
								{#each backups as backup (backup.date)}
									<tr class="table-row">
										<td class="table-cell">
											<div class="font-semibold text-slate-900 dark:text-neutral-100">{formatDate(backup.date)}</div>
											<div class="text-xs text-slate-500 dark:text-neutral-400">{backup.date}</div>
										</td>
										<td class="table-cell">
											{#if backup.encrypted}
												<span class="inline-flex items-center gap-1 text-sm text-emerald-700 dark:text-emerald-400"><Lock size={13} />Encrypted</span>
											{:else}
												<span class="inline-flex items-center gap-1 text-sm text-slate-500 dark:text-neutral-400"><LockOpen size={13} />Plain</span>
											{/if}
										</td>
										<td class="table-cell text-right font-medium text-slate-700 dark:text-neutral-300">{formatSize(backup.size_bytes)}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>

					{#if encrypted && plainCount > 0}
						<div class="flex flex-wrap items-center gap-2 text-sm text-slate-600 dark:text-neutral-400">
							{#if confirmingDeletePlain}
								<span>Delete {plainCount} unencrypted {plainCount === 1 ? 'backup' : 'backups'} permanently?</span>
								<button class="btn-danger-sm" onclick={deletePlain}>Delete</button>
								<button class="btn-secondary-sm" onclick={() => (confirmingDeletePlain = false)}>Cancel</button>
							{:else}
								<span>{plainCount} older {plainCount === 1 ? 'backup is' : 'backups are'} not encrypted.</span>
								<button class="btn-secondary-sm" onclick={() => (confirmingDeletePlain = true)}>Delete unencrypted</button>
							{/if}
						</div>
						{#if deletePlainError}
							<p class="form-error" role="alert">{deletePlainError}</p>
						{/if}
					{/if}
				{/if}
			</div>
		</div>
	{/if}
</section>
