<script lang="ts">
	import {
		Artwork,
		AutoGrid,
		Badge,
		Breadcrumb,
		Button,
		Card,
		Cluster,
		CodeBlock,
		CopyButton,
		Disclosure,
		Field,
		Heading,
		Input,
		KeyValue,
		type KeyValueRow,
		Link,
		Modal,
		SectionHeader,
		Select,
		Skeleton,
		Stack,
		Text,
		Textarea,
		Timestamp,
		toasts,
	} from '@dorsk/tsumikit';
	import { api, type Attachment, type Report, type Status } from '../lib/api.ts';
	import ActorName from '../lib/components/ActorName.svelte';
	import ErrorState from '../lib/components/ErrorState.svelte';
	import EventRow from '../lib/components/EventRow.svelte';
	import StatusBadge from '../lib/components/StatusBadge.svelte';
	import { errorText, formatBytes, kindTone, STATUSES, sentence, statusLabel } from '../lib/labels.ts';
	import { Resource } from '../lib/query.svelte.ts';
	import { projects } from '../lib/session.svelte.ts';

	let { project, id }: { project: string; id: string } = $props();

	const report = new Resource<Report>(() => api.report(project, id));
	report.load();

	const r = $derived(report.data);
	const events = $derived([...(r?.events ?? [])].sort((a, b) => a.at.localeCompare(b.at)));

	let saving = $state<'status' | 'assignee' | 'ticket' | 'claim' | 'note' | null>(null);
	let assignee = $state('');
	let ticket = $state('');
	let note = $state('');
	let zoomed = $state<number | null>(null);

	$effect(() => {
		assignee = r?.assignee ?? '';
		ticket = r?.ticket ?? '';
	});

	async function mutate<T>(what: NonNullable<typeof saving>, run: () => Promise<T>, done: string) {
		if (saving) return;
		saving = what;
		try {
			const result = await run();
			toasts.ok(done);
			projects.load();
			return result;
		} catch (e) {
			toasts.error(errorText(e));
		} finally {
			saving = null;
		}
	}

	async function setStatus(status: Status) {
		if (!r || status === r.status) return;
		const next = await mutate(
			'status',
			() => api.patchReport(project, id, { status }),
			`Status set to ${statusLabel(status).toLowerCase()}`,
		);
		if (next) report.set(next);
		else report.set({ ...r });
	}

	async function saveField(field: 'assignee' | 'ticket', value: string) {
		if (!r) return;
		const next = value.trim() || null;
		if (next === r[field]) return;
		const label = field === 'assignee' ? 'Assignee' : 'Ticket';
		const saved = await mutate(
			field,
			() => api.patchReport(project, id, { [field]: next }),
			next ? `${label} set to ${next}` : `${label} cleared`,
		);
		if (saved) report.set(saved);
	}

	async function releaseClaim() {
		const ok = await mutate(
			'claim',
			async () => {
				await api.releaseClaim(project, id);
				return api.report(project, id);
			},
			'Claim released',
		);
		if (ok) report.set(ok);
	}

	async function addNote() {
		const text = note.trim();
		if (!r || !text) return;
		const event = await mutate('note', () => api.addNote(project, id, text), 'Note added');
		if (event) {
			note = '';
			report.set({ ...r, events: [...r.events, event] });
		}
	}

	const details = $derived.by<KeyValueRow[]>(() => {
		if (!r) return [];
		const client = [r.client_name, r.client_version].filter(Boolean).join(' ');
		return [
			{ label: 'Page', value: pageLink },
			...(r.origin ? [{ label: 'Origin', value: r.origin }] : []),
			...(client ? [{ label: 'Client', value: client }] : []),
			{ label: 'Created', value: createdAt },
			{ label: 'Received', value: receivedAt },
			{ label: 'Report id', value: reportId },
		];
	});

	const zoomedShot = $derived(zoomed === null ? undefined : r?.attachments[zoomed]);
	const shotAlt = (index: number) => `Screenshot ${index + 1} of ${r?.attachments.length ?? 0}`;
	const shotMeta = (a: Attachment) => `${a.width} × ${a.height} · ${formatBytes(a.bytes)}`;
</script>

<svelte:head><title>{project} {id} · hmm admin</title></svelte:head>

{#snippet pageLink()}
	{#if r?.page_url}
		<Link href={r.page_url} target="_blank" rel="noreferrer" underline="always" class="wrap">
			{r.page_url}
		</Link>
	{:else}
		<Text tone="faint">Not sent</Text>
	{/if}
{/snippet}
{#snippet createdAt()}
	<Timestamp value={r?.created_at} mode="datetime" />
{/snippet}
{#snippet receivedAt()}
	<Timestamp value={r?.received_at} mode="datetime" />
{/snippet}
{#snippet reportId()}
	<span class="copy-row">
		<Text mono>{id}</Text>
		<CopyButton text={id} label="Copy report id" variant="ghost" box="xs" />
	</span>
{/snippet}

<Stack gap="var(--sp-4)">
	<Breadcrumb
		size="sm"
		items={[
			{ label: 'Reports', href: '/reports' },
			{ label: project, href: `/reports?project=${encodeURIComponent(project)}` },
			{ label: id },
		]}
	/>

	{#if report.error && !r}
		<ErrorState error={report.error} onretry={() => report.load()} />
	{:else if !r}
		<div class="layout" aria-busy="true">
			<Stack gap="var(--sp-4)">
				<Skeleton width="40%" />
				<Card><Skeleton lines={4} /></Card>
				<Card><Skeleton lines={6} /></Card>
			</Stack>
			<Stack gap="var(--sp-4)">
				<Card><Skeleton lines={5} /></Card>
				<Card><Skeleton lines={6} /></Card>
			</Stack>
		</div>
	{:else}
		<Cluster gap="var(--sp-3)" align="center">
			<Heading level={2} size="lg">
				{sentence(r.kind)}{#if r.category}<Text tone="muted" size="lg">{' · '}{r.category}</Text>{/if}
			</Heading>
			<StatusBadge status={r.status} size="md" />
			{#if r.kind !== 'feedback'}<Badge tone={kindTone(r.kind)} size="md">{sentence(r.kind)}</Badge>{/if}
			<Text tone="muted" size="sm">
				received <Timestamp value={r.received_at} mode="relative" tone="muted" size="sm" />
			</Text>
		</Cluster>

		<div class="layout">
			<Stack gap="var(--sp-5)">
				<Card>
					{#if r.comment.trim()}
						<Text as="p" measure class="comment">{r.comment}</Text>
					{:else}
						<Text as="p" tone="faint" italic>The report was sent without a comment.</Text>
					{/if}
				</Card>

				{#if r.attachments.length > 0}
					<Stack gap="var(--sp-3)">
						<SectionHeader title="Screenshots" level={3} size="md" count={r.attachments.length} />
						<AutoGrid min="12rem" gap="var(--sp-3)">
							{#each r.attachments as shot, index (shot.id)}
								<Card
									as="button"
									padding="none"
									overflow="hidden"
									interactive
									aria-label="View {shotAlt(index).toLowerCase()} large"
									onclick={() => (zoomed = index)}
								>
									<Artwork
										src={api.attachmentUrl(project, id, shot.id)}
										alt={shotAlt(index)}
										aspect="16/10"
										fit="cover"
										radius="sm"
										fallback="icon"
										icon="image"
									/>
									<span class="shot-meta"><Text size="xs" tone="muted" numeric>{shotMeta(shot)}</Text></span>
								</Card>
							{/each}
						</AutoGrid>
					</Stack>
				{/if}

				<Stack gap="var(--sp-3)">
					<SectionHeader title="Activity" level={3} size="md" count={events.length} />
					<Card padding="none">
						<ol class="timeline" aria-label="Activity">
							{#each events as event, index (index)}
								<EventRow {event} />
							{/each}
						</ol>
						{#snippet footer()}
							<Stack gap="var(--sp-2)" class="composer">
								<Textarea
									bind:value={note}
									rows={2}
									autoresize
									maxHeight="16rem"
									maxlength={4000}
									placeholder="Write a note for whoever picks this up next"
									aria-label="Note"
									submitOn="mod-enter"
									onsubmit={addNote}
								/>
								<Cluster justify="flex-end">
									<Button
										variant="primary"
										size="sm"
										disabled={!note.trim()}
										loading={saving === 'note'}
										onclick={addNote}
									>
										Add note
									</Button>
								</Cluster>
							</Stack>
						{/snippet}
					</Card>
				</Stack>

				<Card padding="none" overflow="hidden">
					<Disclosure>
						{#snippet header()}<Text weight="medium">Envelope JSON</Text>{/snippet}
						<CodeBlock code={JSON.stringify(r.envelope, null, 2)} lang="json" maxHeight="32rem" />
					</Disclosure>
				</Card>
			</Stack>

			<Stack gap="var(--sp-4)">
				<Card title="Triage" gap="var(--sp-3)">
					<Field label="Status">
						<Select
							value={r.status}
							disabled={saving === 'status'}
							options={STATUSES.map((s) => ({ value: s.id, label: s.label }))}
							onchange={(e) => setStatus(e.currentTarget.value as Status)}
						/>
					</Field>
					<Field label="Assignee">
						<div class="inline-edit">
							<Input
								bind:value={assignee}
								placeholder="Nobody"
								maxlength={200}
								grow
								onenter={() => saveField('assignee', assignee)}
							/>
							{#if assignee.trim() !== (r.assignee ?? '')}
								<Button loading={saving === 'assignee'} onclick={() => saveField('assignee', assignee)}>
									Save
								</Button>
							{/if}
						</div>
					</Field>
					<Field label="Ticket">
						<div class="inline-edit">
							<Input
								bind:value={ticket}
								placeholder="None"
								maxlength={200}
								mono
								grow
								onenter={() => saveField('ticket', ticket)}
							/>
							{#if ticket.trim() !== (r.ticket ?? '')}
								<Button loading={saving === 'ticket'} onclick={() => saveField('ticket', ticket)}>
									Save
								</Button>
							{/if}
						</div>
					</Field>
					<Field label="Claim">
						{#if r.claim}
							<div class="claim">
								<Text size="sm">
									<ActorName actor={r.claim.by} /> until
									<Timestamp value={r.claim.until} mode="time" size="sm" />
								</Text>
								<Button size="sm" loading={saving === 'claim'} onclick={releaseClaim}>Release claim</Button>
							</div>
						{:else}
							<Text size="sm" tone="muted">Nobody holds this report.</Text>
						{/if}
					</Field>
				</Card>

				<Card title="Details">
					<KeyValue rows={details} columns={1} />
				</Card>
			</Stack>
		</div>
	{/if}
</Stack>

{#if zoomedShot && zoomed !== null && r}
	{@const index = zoomed}
	<Modal title={shotAlt(index)} size="xl" onclose={() => (zoomed = null)}>
		{#snippet body()}
			<img
				class="zoomed"
				src={api.attachmentUrl(project, id, zoomedShot.id)}
				alt={shotAlt(index)}
				width={zoomedShot.width}
				height={zoomedShot.height}
			/>
		{/snippet}
		{#snippet footer()}
			<Text size="sm" tone="muted" numeric grow>{shotMeta(zoomedShot)}</Text>
			{#if r.attachments.length > 1}
				<Button size="sm" disabled={index === 0} onclick={() => (zoomed = index - 1)}>Previous</Button>
				<Button
					size="sm"
					disabled={index === r.attachments.length - 1}
					onclick={() => (zoomed = index + 1)}
				>
					Next
				</Button>
			{/if}
			<Button
				size="sm"
				href={api.attachmentUrl(project, id, zoomedShot.id)}
				target="_blank"
				rel="noreferrer"
			>
				Open original
			</Button>
		{/snippet}
	</Modal>
{/if}

<style>
	.layout {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: var(--sp-5);
		align-items: start;
	}
	@media (min-width: 64rem) {
		.layout {
			grid-template-columns: minmax(0, 1fr) 21rem;
		}
	}
	.layout :global(.comment) {
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
	.layout :global(.wrap) {
		overflow-wrap: anywhere;
	}
	:global(.composer) {
		padding: var(--sp-3) var(--sp-4);
	}
	.timeline {
		list-style: none;
		margin: 0;
		padding: var(--sp-2) 0;
	}
	.shot-meta {
		display: block;
		padding: var(--sp-2) var(--sp-3);
		text-align: start;
	}
	.inline-edit,
	.claim,
	.copy-row {
		display: flex;
		align-items: center;
		gap: var(--sp-2);
		min-width: 0;
	}
	.claim {
		justify-content: space-between;
		flex-wrap: wrap;
	}
	.zoomed {
		display: block;
		max-width: 100%;
		height: auto;
		margin-inline: auto;
		border-radius: var(--r-sm);
	}
</style>
