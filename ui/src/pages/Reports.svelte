<script lang="ts">
	import {
		Button,
		Callout,
		Cluster,
		type Column,
		DataTable,
		EmptyState,
		FilterChips,
		Icon,
		Input,
		Link,
		LoadMore,
		mq,
		SectionHeader,
		Select,
		Stack,
		Text,
		Timestamp,
	} from '@dorsk/tsumikit';
	import { untrack } from 'svelte';
	import { api, type ReportSummary, type Status } from '../lib/api.ts';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import StatusBadge from '../lib/components/StatusBadge.svelte';
	import {
		errorText,
		KINDS,
		OPEN_STATUSES,
		pagePath,
		parseActor,
		plural,
		STATUSES,
		sentence,
	} from '../lib/labels.ts';
	import { Paged } from '../lib/query.svelte.ts';
	import { reportHref, router } from '../lib/router.svelte.ts';
	import { projects } from '../lib/session.svelte.ts';

	const PAGE_SIZE = 30;

	const filter = $derived({
		project: router.query.get('project'),
		status: router.query.get('status') ?? 'open',
		kind: router.query.get('kind'),
		q: router.query.get('q') ?? '',
	});
	const filtered = $derived(
		Boolean(filter.project || filter.kind || filter.q || filter.status !== 'open'),
	);

	const list = new Paged<ReportSummary>(async (cursor) => {
		const { status, ...rest } = filter;
		const page = await api.reports(
			{ ...rest, status: status === 'all' ? null : status },
			cursor,
			PAGE_SIZE,
		);
		return { items: page.reports, next: page.next_cursor };
	});

	$effect(() => {
		void filter;
		untrack(() => {
			list.reload();
			projects.load();
		});
	});

	let search = $state('');
	let timer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		search = filter.q;
	});
	function onsearch(value: string) {
		search = value;
		clearTimeout(timer);
		timer = setTimeout(() => router.setQuery({ q: value.trim() }), 250);
	}

	const counts = $derived.by(() => {
		const scope = (projects.data ?? []).filter(
			(p) => !filter.project || p.project === filter.project,
		);
		const of = (statuses: Status[]) =>
			scope.reduce((n, p) => n + statuses.reduce((m, s) => m + (p.counts[s] ?? 0), 0), 0);
		return projects.data ? of : undefined;
	});
	const chips = $derived([
		{ id: 'open', label: 'Open', count: counts?.(OPEN_STATUSES) },
		...STATUSES.map((s) => ({ id: s.id, label: s.label, count: counts?.([s.id]) })),
		{ id: 'all', label: 'All', count: counts?.(STATUSES.map((s) => s.id)) },
	]);
	const projectOptions = $derived([
		{ value: '', label: 'All projects' },
		...(projects.data ?? []).map((p) => ({
			value: p.project,
			label: p.project,
			hint: p.counts.new ? `${p.counts.new} new` : undefined,
		})),
	]);
	const kindOptions = [
		{ value: '', label: 'All kinds' },
		...KINDS.map((k) => ({ value: k, label: sentence(k) })),
	];

	const columns = $derived<Column<ReportSummary>[]>([
		{ key: 'comment', label: 'Report', role: 'title' },
		{ key: 'status', label: 'Status', width: '8rem', nowrap: true, role: 'meta', stackLabel: false },
		{ key: 'kind', label: 'Kind', width: '9.5rem', nowrap: true, role: 'meta', stackLabel: false },
		...(filter.project
			? []
			: [
					{
						key: 'project',
						label: 'Project',
						width: '7rem',
						truncate: true,
						role: 'meta',
						stackLabel: false,
					} satisfies Column<ReportSummary>,
				]),
		{ key: 'claim', label: 'Claimed by', width: '9rem', truncate: true, role: 'detail', hideBelow: 'lg' },
		{ key: 'ticket', label: 'Ticket', width: '6.5rem', truncate: true, role: 'detail', hideBelow: 'md' },
		{ key: 'received', label: 'Received', width: '7rem', nowrap: true, align: 'right', role: 'meta', stackLabel: false },
	]);

	const title = $derived(filter.project ?? 'All projects');
</script>

<svelte:head><title>Reports · {title} · hmm admin</title></svelte:head>

{#snippet colComment(r: ReportSummary)}
	<span class="report">
		<Link href={reportHref(r.project, r.id)} tone="inherit" class="comment">
			{#if r.comment.trim()}{r.comment}{:else}<Text tone="faint" italic>No comment</Text>{/if}
		</Link>
		<span class="sub">
			{#if r.page_url}<Text size="xs" tone="muted" truncate>{pagePath(r.page_url)}</Text>{/if}
			{#if r.attachments > 0}
				<span class="shots" title={plural(r.attachments, 'screenshot')}>
					<Icon name="image" size={12} />
					<Text size="xs" tone="muted" numeric>{r.attachments}</Text>
					<span class="sr-only">{plural(r.attachments, 'screenshot')}</span>
				</span>
			{/if}
		</span>
	</span>
{/snippet}
{#snippet colStatus(r: ReportSummary)}
	<StatusBadge status={r.status} />
{/snippet}
{#snippet colKind(r: ReportSummary)}
	<Text size="sm" tone={r.kind === 'error' ? 'danger' : r.kind === 'alert' ? 'warn' : 'default'}>
		{sentence(r.kind)}
	</Text>{#if r.category}<Text size="sm" tone="muted">{' · '}{r.category}</Text>{/if}
{/snippet}
{#snippet colProject(r: ReportSummary)}
	<Text size="sm" tone="muted">{r.project}</Text>
{/snippet}
{#snippet colClaim(r: ReportSummary)}
	{#if r.claim}
		<span class="claim" title="Claimed by {r.claim.by}">
			<Icon name="lock" size={12} />
			<Text size="sm" truncate>{parseActor(r.claim.by).name}</Text>
		</span>
	{/if}
{/snippet}
{#snippet colTicket(r: ReportSummary)}
	{#if r.ticket}<Text size="sm" mono>{r.ticket}</Text>{/if}
{/snippet}
{#snippet colReceived(r: ReportSummary)}
	<Timestamp value={r.received_at} mode="relative" short size="sm" tone="muted" details={false} />
{/snippet}

<Stack gap="var(--sp-4)">
	<SectionHeader {title} level={2} size="lg" subtitle="Feedback sent by the hmm widget, newest first." />

	<Cluster gap="var(--sp-3)" align="center" justify="space-between">
		<FilterChips
			label="Filter by status"
			options={chips}
			value={filter.status}
			href={(id) => router.href({ status: id === 'open' ? null : id })}
			size="sm"
		/>
		<Cluster gap="var(--sp-2)" align="center">
			{#if !mq.md}
				<Select
					size="sm"
					width="auto"
					aria-label="Project"
					options={projectOptions}
					value={filter.project ?? ''}
					onchange={(e) => router.setQuery({ project: e.currentTarget.value })}
				/>
			{/if}
			<Select
				size="sm"
				width="auto"
				aria-label="Kind"
				options={kindOptions}
				value={filter.kind ?? ''}
				onchange={(e) => router.setQuery({ kind: e.currentTarget.value })}
			/>
			<Input
				size="sm"
				type="search"
				icon="search"
				width={mq.sm ? '16rem' : '100%'}
				placeholder="Search comment, page or id"
				aria-label="Search reports"
				clearable
				value={search}
				oninput={(e) => onsearch(e.currentTarget.value)}
				onclear={() => onsearch('')}
			/>
		</Cluster>
	</Cluster>

	{#if list.error && list.items.length === 0}
		<ErrorState error={list.error} onretry={() => list.reload()} />
	{:else}
		{#if list.error}
			<Callout tone="danger" title="The list could not be refreshed">
				{errorText(list.error)}
				{#snippet actions()}<Button size="sm" onclick={() => list.reload()}>Try again</Button>{/snippet}
			</Callout>
		{/if}
		<DataTable
			{columns}
			rows={list.items}
			rowKey={(r) => `${r.project}/${r.id}`}
			layout="fixed"
			responsive="stack"
			style="--dt-meta-font: var(--font-sans)"
			loading={list.loading && !list.loaded}
			loadingRows={8}
			loadingLabel="Loading reports"
			onrowclick={(r) => router.navigate(reportHref(r.project, r.id))}
			cellSnippets={{
				comment: colComment,
				status: colStatus,
				kind: colKind,
				project: colProject,
				claim: colClaim,
				ticket: colTicket,
				received: colReceived,
			}}
		>
			{#snippet empty()}
				{#if filtered}
					<EmptyState
						size="compact"
						icon="filter"
						title="No report matches these filters"
						description="Try another status, or clear the filters to see everything that is open."
					>
						{#snippet action()}<Button href="/reports">Clear filters</Button>{/snippet}
					</EmptyState>
				{:else}
					<EmptyState
						size="compact"
						icon="check-circle"
						title="Nothing is open"
						description="Every report has been resolved or dismissed. New ones show up here as they arrive."
					>
						{#snippet action()}<Button href={router.href({ status: 'all' })}>Show all reports</Button>{/snippet}
					</EmptyState>
				{/if}
			{/snippet}
		</DataTable>
		{#if list.items.length > 0 && list.more !== 'done'}
			<LoadMore state={list.more} onload={() => list.loadMore()} label="Load more reports" />
		{/if}
	{/if}
</Stack>

<style>
	.report {
		display: flex;
		flex-direction: column;
		gap: var(--sp-1);
		min-width: 0;
	}
	.report :global(.comment) {
		display: block;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sub {
		display: flex;
		align-items: center;
		gap: var(--sp-3);
		min-width: 0;
		color: var(--text-muted);
	}
	.shots,
	.claim {
		display: inline-flex;
		align-items: center;
		gap: var(--sp-1);
		min-width: 0;
		flex: none;
		color: var(--text-muted);
	}
	.claim {
		flex: 0 1 auto;
		max-width: 100%;
	}
</style>
