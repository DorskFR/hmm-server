<script lang="ts">
	import {
		Badge,
		Button,
		Callout,
		Checkbox,
		Cluster,
		type Column,
		ConfirmModal,
		DataTable,
		EmptyState,
		Field,
		Input,
		Modal,
		type RowAction,
		Select,
		Stack,
		Switch,
		Text,
		Timestamp,
		toasts,
	} from '@dorsk/tsumikit';
	import { api, type Webhook } from '../lib/api.ts';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import SecretModal from '../lib/components/SecretModal.svelte';
	import WebhooksHeader from '../lib/components/WebhooksHeader.svelte';
	import { errorText, WEBHOOK_EVENTS } from '../lib/labels.ts';
	import { Resource } from '../lib/query.svelte.ts';
	import { router } from '../lib/router.svelte.ts';
	import { projects } from '../lib/session.svelte.ts';

	const hooks = new Resource(async () => (await api.webhooks()).webhooks);
	hooks.load();

	let editing = $state<Webhook | 'new' | null>(null);
	let submitting = $state(false);
	let formError = $state<unknown>(undefined);
	let name = $state('');
	let project = $state('');
	let url = $state('');
	let events = $state<Record<string, boolean>>({});
	let created = $state<{ name: string; secret: string } | null>(null);
	let deleting = $state<Webhook | null>(null);
	let toggling = $state<string | null>(null);

	const pickedEvents = $derived(WEBHOOK_EVENTS.map((e) => e.id).filter((e) => events[e]));
	const valid = $derived(
		name.trim().length > 0 &&
			url.trim().length > 0 &&
			pickedEvents.length > 0 &&
			(editing !== 'new' || project.length > 0),
	);
	const projectOptions = $derived(
		(projects.data ?? []).map((p) => ({ value: p.project, label: p.project })),
	);
	const deliveriesHref = (hook: Webhook) =>
		`/webhooks/deliveries?webhook=${encodeURIComponent(hook.id)}`;

	function openForm(hook: Webhook | 'new') {
		const existing = hook === 'new' ? null : hook;
		name = existing?.name ?? '';
		project = existing?.project ?? projects.data?.[0]?.project ?? '';
		url = existing?.url ?? '';
		const on = existing?.events ?? ['report.created'];
		events = Object.fromEntries(WEBHOOK_EVENTS.map((e) => [e.id, on.includes(e.id)]));
		formError = undefined;
		editing = hook;
	}

	async function submit() {
		if (!valid || submitting || !editing) return;
		submitting = true;
		formError = undefined;
		const body = { name: name.trim(), url: url.trim(), events: pickedEvents };
		try {
			if (editing === 'new') {
				const hook = await api.createWebhook({ ...body, project });
				created = { name: hook.name, secret: hook.secret };
			} else {
				await api.patchWebhook(editing.id, body);
				toasts.ok(`Webhook ${body.name} saved`);
			}
			editing = null;
			hooks.load();
		} catch (e) {
			formError = e;
		} finally {
			submitting = false;
		}
	}

	async function toggle(hook: Webhook) {
		if (toggling) return;
		toggling = hook.id;
		try {
			await api.patchWebhook(hook.id, { enabled: !hook.enabled });
			toasts.ok(`Webhook ${hook.name} ${hook.enabled ? 'disabled' : 'enabled'}`);
		} catch (e) {
			toasts.error(errorText(e));
		} finally {
			toggling = null;
			await hooks.load();
		}
	}

	async function ping(hook: Webhook) {
		try {
			await api.pingWebhook(hook.id);
			toasts.show(`Ping queued for ${hook.name}`, {
				tone: 'ok',
				action: { label: 'View deliveries', run: () => router.navigate(deliveriesHref(hook)) },
			});
		} catch (e) {
			toasts.error(errorText(e));
		}
	}

	async function remove(hook: Webhook) {
		await api.deleteWebhook(hook.id);
		deleting = null;
		toasts.ok(`Webhook ${hook.name} deleted`);
		await hooks.load();
	}

	const columns: Column<Webhook>[] = [
		{ key: 'name', label: 'Webhook', role: 'title' },
		{ key: 'events', label: 'Events', width: '15rem', role: 'detail', hideBelow: 'lg' },
		{ key: 'last', label: 'Last delivery', width: '11rem', role: 'detail' },
		{ key: 'enabled', label: 'Enabled', width: '6rem', role: 'detail' },
	];

	const actions = (hook: Webhook): RowAction[] => [
		{ id: 'ping', label: `Ping ${hook.name}`, icon: 'zap', onselect: () => ping(hook) },
		{
			id: 'deliveries',
			label: 'Deliveries',
			icon: 'list',
			onselect: () => router.navigate(deliveriesHref(hook)),
		},
		{ id: 'edit', label: 'Edit', icon: 'edit', onselect: () => openForm(hook) },
		{ id: 'delete', label: 'Delete', icon: 'trash', danger: true, onselect: () => (deleting = hook) },
	];
</script>

<svelte:head><title>Webhooks · hmm admin</title></svelte:head>

{#snippet colName(hook: Webhook)}
	<span class="hook">
		<span class="hook-name">
			<Text weight="medium" tone={hook.enabled ? 'default' : 'muted'} truncate>{hook.name}</Text>
			<Badge size="xs" tone="muted">{hook.project}</Badge>
		</span>
		<Text size="xs" tone="muted" mono truncate title={hook.url}>{hook.url}</Text>
	</span>
{/snippet}
{#snippet colEvents(hook: Webhook)}
	<Cluster gap="var(--sp-1)">
		{#each hook.events as event (event)}
			<Badge size="xs" mono>{event}</Badge>
		{/each}
	</Cluster>
{/snippet}
{#snippet colLast(hook: Webhook)}
	{#if hook.last_delivery}
		{@const last = hook.last_delivery}
		{@const ok = !last.error && (last.status_code ?? 0) >= 200 && (last.status_code ?? 0) < 300}
		<span class="last" title={last.error ?? undefined}>
			<Badge size="xs" tone={ok ? 'ok' : 'danger'} icon={ok ? 'check' : 'alert-circle'} numeric>
				{last.status_code ?? 'Failed'}
			</Badge>
			<Timestamp value={last.at} mode="relative" short size="sm" tone="muted" details={false} />
		</span>
	{:else}
		<Text size="sm" tone="muted">Never</Text>
	{/if}
{/snippet}
{#snippet colEnabled(hook: Webhook)}
	<Switch
		size="sm"
		checked={hook.enabled}
		label="{hook.name} enabled"
		disabled={toggling === hook.id}
		onclick={() => toggle(hook)}
	/>
{/snippet}

<Stack gap="var(--sp-4)">
	<WebhooksHeader tab="endpoints">
		{#snippet actions()}
			<Button variant="primary" size="sm" onclick={() => openForm('new')}>New webhook</Button>
		{/snippet}
	</WebhooksHeader>

	{#if hooks.error && !hooks.data}
		<ErrorState error={hooks.error} onretry={() => hooks.load()} />
	{:else}
		<DataTable
			{columns}
			rows={hooks.data ?? []}
			rowKey={(h) => h.id}
			layout="fixed"
			responsive="stack"
			stackActions="end"
			style="--dt-meta-font: var(--font-sans)"
			loading={!hooks.data}
			loadingRows={3}
			loadingLabel="Loading webhooks"
			{actions}
			cellSnippets={{ name: colName, events: colEvents, last: colLast, enabled: colEnabled }}
		>
			{#snippet empty()}
				<EmptyState
					size="compact"
					icon="send"
					title="No webhook yet"
					description="A webhook tells another service about new reports the moment they arrive, so an agent does not have to poll."
				>
					{#snippet action()}<Button onclick={() => openForm('new')}>New webhook</Button>{/snippet}
				</EmptyState>
			{/snippet}
		</DataTable>
	{/if}
</Stack>

{#if editing}
	{@const isNew = editing === 'new'}
	<Modal
		title={isNew ? 'New webhook' : 'Edit webhook'}
		size="md"
		busy={submitting}
		onclose={() => (editing = null)}
	>
		{#snippet body()}
			<Stack gap="var(--sp-4)">
				{#if formError}
					<Callout tone="danger" title="The webhook was not saved">{errorText(formError)}</Callout>
				{/if}
				<Field label="Name">
					<Input bind:value={name} placeholder="cctui-dispatch" autocomplete="off" />
				</Field>
				{#if isNew}
					<Field label="Project" hint="Only reports of this project are sent.">
						<Select bind:value={project} options={projectOptions} />
					</Field>
				{/if}
				<Field label="URL" hint="Must be https. Redirects are not followed.">
					<Input
						bind:value={url}
						type="url"
						mono
						placeholder="https://example.com/hooks/hmm"
						autocomplete="off"
						onenter={submit}
					/>
				</Field>
				<Field label="Events">
					<Stack gap="var(--sp-3)">
						{#each WEBHOOK_EVENTS as event (event.id)}
							<div>
								<Checkbox label={event.label} bind:checked={events[event.id]} />
								<Text as="p" size="xs" tone="muted" class="help">{event.help}</Text>
							</div>
						{/each}
					</Stack>
				</Field>
			</Stack>
		{/snippet}
		{#snippet footer()}
			<Button onclick={() => (editing = null)}>Cancel</Button>
			<Button variant="primary" disabled={!valid} loading={submitting} onclick={submit}>
				{isNew ? 'Create webhook' : 'Save changes'}
			</Button>
		{/snippet}
	</Modal>
{/if}

{#if created}
	<SecretModal
		title="Webhook {created.name} created"
		label="Signing secret"
		value={created.secret}
		warning="Copy the signing secret now. The receiver needs it to verify x-hmm-signature, and it cannot be shown again."
		onclose={() => (created = null)}
	/>
{/if}

{#if deleting}
	{@const hook = deleting}
	<ConfirmModal
		open
		tone="danger"
		title="Delete {hook.name}?"
		message="It stops receiving events at once, and its signing secret is lost. This cannot be undone."
		confirmLabel="Delete webhook"
		onconfirm={() => remove(hook)}
		oncancel={() => (deleting = null)}
	/>
{/if}

<style>
	.hook {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.hook-name,
	.last {
		display: flex;
		align-items: center;
		gap: var(--sp-2);
		min-width: 0;
	}
	div :global(.help) {
		margin: 0;
		padding-inline-start: var(--sp-6);
	}
</style>
