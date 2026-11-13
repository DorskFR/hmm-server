<script lang="ts">
	import {
		Badge,
		Button,
		Cluster,
		type Column,
		DataTable,
		EmptyState,
		Link,
		LoadMore,
		type RowAction,
		Select,
		Stack,
		Text,
		Timestamp,
		toasts,
	} from '@dorsk/tsumikit';
	import { untrack } from 'svelte';
	import { api, type Delivery } from '../lib/api.ts';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import WebhooksHeader from '../lib/components/WebhooksHeader.svelte';
	import { errorText, sentence } from '../lib/labels.ts';
	import { Paged, Resource } from '../lib/query.svelte.ts';
	import { reportHref, router } from '../lib/router.svelte.ts';

	const PAGE_SIZE = 50;
	const TONES = { pending: 'warn', delivered: 'ok', failed: 'danger' } as const;
	const ICONS = { pending: 'clock', delivered: 'check', failed: 'alert-circle' } as const;

	const hooks = new Resource(async () => (await api.webhooks()).webhooks);
	hooks.load();

	const webhook = $derived(router.query.get('webhook'));
	const list = new Paged<Delivery>(async (cursor) => {
		const page = await api.deliveries(webhook, cursor, PAGE_SIZE);
		return { items: page.deliveries, next: page.next_cursor };
	});
	$effect(() => {
		void webhook;
		untrack(() => list.reload());
	});

	const hookOptions = $derived([
		{ value: '', label: 'All webhooks' },
		...(hooks.data ?? []).map((h) => ({ value: h.id, label: h.name })),
	]);

	async function redeliver(delivery: Delivery) {
		try {
			const queued = await api.redeliver(delivery.id);
			list.replace((d) => d.id === queued.id, queued);
			toasts.ok('A new attempt was queued');
		} catch (e) {
			toasts.error(errorText(e));
		}
	}

	const columns: Column<Delivery>[] = [
		{ key: 'event', label: 'Event', role: 'title' },
		{ key: 'webhook', label: 'Webhook', width: '10rem', truncate: true, role: 'meta', stackLabel: false },
		{ key: 'state', label: 'State', width: '8rem', nowrap: true, role: 'meta', stackLabel: false },
		{ key: 'attempts', label: 'Attempts', width: '6rem', align: 'right', role: 'detail', hideBelow: 'md' },
		{ key: 'response', label: 'Last response', width: '16rem', role: 'detail' },
		{ key: 'updated', label: 'Updated', width: '7rem', nowrap: true, align: 'right', role: 'meta', stackLabel: false },
	];

	const actions = (delivery: Delivery): RowAction[] => [
		{ id: 'redeliver', label: 'Redeliver', icon: 'retry', onselect: () => redeliver(delivery) },
	];

	const stateOf = (delivery: Delivery) =>
		delivery.state in TONES ? (delivery.state as keyof typeof TONES) : 'pending';
</script>

<svelte:head><title>Webhook deliveries · hmm admin</title></svelte:head>

{#snippet colEvent(d: Delivery)}
	<span class="event">
		<Text size="sm" mono>{d.event}</Text>
		{#if d.project && d.report_id}
			<Link href={reportHref(d.project, d.report_id)} tone="muted">
				<Text size="xs" mono>{d.project}/{d.report_id}</Text>
			</Link>
		{/if}
	</span>
{/snippet}
{#snippet colWebhook(d: Delivery)}
	<Text size="sm">{d.webhook_name}</Text>
{/snippet}
{#snippet colState(d: Delivery)}
	<Badge size="sm" tone={TONES[stateOf(d)]} icon={ICONS[stateOf(d)]}>{sentence(d.state)}</Badge>
{/snippet}
{#snippet colAttempts(d: Delivery)}
	<Text size="sm" numeric>{d.attempts}</Text>
{/snippet}
{#snippet colResponse(d: Delivery)}
	<span class="response">
		{#if d.last_status_code !== null}
			<Text size="sm" numeric>
				HTTP {d.last_status_code}{#if d.last_duration_ms !== null}
					<Text size="sm" tone="muted" numeric>{' '}in {d.last_duration_ms} ms</Text>{/if}
			</Text>
		{/if}
		{#if d.last_error}<Text size="sm" tone="danger">{d.last_error}</Text>{/if}
		{#if d.state === 'pending' && d.next_attempt_at}
			<Text size="xs" tone="muted">
				Next attempt <Timestamp value={d.next_attempt_at} mode="relative" size="xs" tone="muted" />
			</Text>
		{/if}
	</span>
{/snippet}
{#snippet colUpdated(d: Delivery)}
	<Timestamp value={d.updated_at} mode="relative" short size="sm" tone="muted" details={false} />
{/snippet}

<Stack gap="var(--sp-4)">
	<WebhooksHeader tab="deliveries" />

	<Cluster gap="var(--sp-2)" align="center" justify="space-between">
		<Select
			size="sm"
			width="auto"
			aria-label="Webhook"
			options={hookOptions}
			value={webhook ?? ''}
			onchange={(e) => router.setQuery({ webhook: e.currentTarget.value })}
		/>
		<Button size="sm" loading={list.loading && list.loaded} onclick={() => list.reload()}>Refresh</Button>
	</Cluster>

	{#if list.error && list.items.length === 0}
		<ErrorState error={list.error} onretry={() => list.reload()} />
	{:else}
		<DataTable
			{columns}
			rows={list.items}
			rowKey={(d) => d.id}
			layout="fixed"
			responsive="stack"
			stackActions="end"
			style="--dt-meta-font: var(--font-sans)"
			loading={list.loading && !list.loaded}
			loadingRows={8}
			loadingLabel="Loading deliveries"
			{actions}
			cellSnippets={{
				event: colEvent,
				webhook: colWebhook,
				state: colState,
				attempts: colAttempts,
				response: colResponse,
				updated: colUpdated,
			}}
		>
			{#snippet empty()}
				<EmptyState
					size="compact"
					icon="send"
					title="Nothing has been delivered yet"
					description="Deliveries appear here when a report arrives for a project with a webhook. Ping a webhook to send a test now."
				>
					{#snippet action()}<Button href="/webhooks">Go to endpoints</Button>{/snippet}
				</EmptyState>
			{/snippet}
		</DataTable>
		{#if list.items.length > 0 && list.more !== 'done'}
			<LoadMore state={list.more} onload={() => list.loadMore()} label="Load more deliveries" />
		{/if}
	{/if}
</Stack>

<style>
	.event,
	.response {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
</style>
