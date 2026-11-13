<script lang="ts">
	import { Icon, type IconName, Text, Timestamp } from '@dorsk/tsumikit';
	import type { ReportEvent } from '../api.ts';
	import { formatBytes, plural, statusLabel } from '../labels.ts';
	import ActorName from './ActorName.svelte';

	let { event }: { event: ReportEvent } = $props();

	const ICONS: Record<string, IconName> = {
		received: 'mail',
		status: 'tag',
		assigned: 'user',
		ticket: 'link',
		claimed: 'lock',
		released: 'unlock',
		note: 'file-text',
		webhook: 'send',
	};

	const data = $derived(event.data ?? {});
	const str = (key: string) => (typeof data[key] === 'string' ? (data[key] as string) : null);
	const num = (key: string) => (typeof data[key] === 'number' ? (data[key] as number) : null);

	const webhookFailed = $derived(
		event.type === 'webhook' &&
			(str('error') !== null || (num('status_code') ?? 200) < 200 || (num('status_code') ?? 200) > 299),
	);
	const known = $derived(event.type in ICONS);
</script>

<li class="event" data-type={event.type}>
	<span class="glyph" class:failed={webhookFailed}><Icon name={ICONS[event.type] ?? 'info'} size={14} /></span>
	<div class="body">
		<Text size="sm">
			{#if event.type === 'received'}
				Report received{#if num('attachments')}{' '}with {plural(num('attachments') ?? 0, 'screenshot')}{/if}
				{#if num('bytes')}<Text tone="muted" size="sm">({formatBytes(num('bytes') ?? 0)})</Text>{/if}
			{:else if event.type === 'status'}
				<ActorName actor={event.actor} /> changed the status from
				{statusLabel(str('from') ?? 'unknown').toLowerCase()} to
				<Text weight="medium">{statusLabel(str('to') ?? 'unknown').toLowerCase()}</Text>
			{:else if event.type === 'assigned'}
				<ActorName actor={event.actor} />
				{#if str('to')}
					assigned this to <Text weight="medium">{str('to')}</Text>
				{:else}
					removed the assignee{#if str('from')}{' '}({str('from')}){/if}
				{/if}
			{:else if event.type === 'ticket'}
				<ActorName actor={event.actor} />
				{#if str('to')}
					linked ticket <Text mono>{str('to')}</Text>
				{:else}
					removed the ticket{#if str('from')}{' '}<Text mono>{str('from')}</Text>{/if}
				{/if}
			{:else if event.type === 'claimed'}
				<ActorName actor={event.actor} /> claimed this
				{#if str('until')}until <Timestamp value={str('until')} mode="time" size="sm" />{/if}
			{:else if event.type === 'released'}
				<ActorName actor={event.actor} /> released the claim
			{:else if event.type === 'note'}
				<ActorName actor={event.actor} /> wrote a note
			{:else if event.type === 'webhook'}
				Webhook <Text weight="medium">{str('webhook') ?? 'unknown'}</Text>
				{webhookFailed ? 'failed to deliver' : 'delivered'}
				<Text mono>{str('event') ?? 'an event'}</Text>
				<Text tone={webhookFailed ? 'danger' : 'muted'} size="sm">
					({[
						num('attempt') ? `attempt ${num('attempt')}` : null,
						num('status_code') ? `HTTP ${num('status_code')}` : null,
						str('error'),
					]
						.filter(Boolean)
						.join(', ')})
				</Text>
			{:else}
				<ActorName actor={event.actor} /> {event.type}
			{/if}
		</Text>
		{#if event.type === 'note' && str('note')}
			<Text as="p" size="sm" class="note">{str('note')}</Text>
		{:else if !known && Object.keys(data).length > 0}
			<Text as="p" size="xs" tone="muted" mono class="note">{JSON.stringify(data)}</Text>
		{/if}
	</div>
	<Timestamp value={event.at} mode="relative" short size="xs" tone="muted" />
</li>

<style>
	.event {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) auto;
		align-items: baseline;
		gap: var(--sp-3);
		padding: var(--sp-2) var(--sp-4);
	}
	.glyph {
		display: inline-flex;
		align-self: start;
		padding-block-start: var(--sp-1);
		color: var(--text-muted);
	}
	.glyph.failed {
		color: var(--danger);
	}
	.body {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: var(--sp-1);
	}
	.body :global(.note) {
		margin: 0;
		padding: var(--sp-2) var(--sp-3);
		border-inline-start: 2px solid var(--border);
		background: var(--bg-elevated);
		border-radius: var(--r-sm);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
</style>
