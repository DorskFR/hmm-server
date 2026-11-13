<script lang="ts">
	import {
		AppShell,
		Heading,
		Icon,
		Link,
		NavItem,
		NavSection,
		Text,
		ThemePicker,
		Toaster,
	} from '@dorsk/tsumikit';
	import { ApiError } from './lib/api.ts';
	import ErrorState from './lib/components/ErrorState.svelte';
	import { OPEN_STATUSES, parseActor, plural } from './lib/labels.ts';
	import { routeLinks, router } from './lib/router.svelte.ts';
	import { me, projects } from './lib/session.svelte.ts';
	import AccessLog from './pages/AccessLog.svelte';
	import Deliveries from './pages/Deliveries.svelte';
	import Keys from './pages/Keys.svelte';
	import NotFound from './pages/NotFound.svelte';
	import Report from './pages/Report.svelte';
	import Reports from './pages/Reports.svelte';
	import Webhooks from './pages/Webhooks.svelte';

	me.load();
	projects.load();

	const seg = $derived(router.segments);
	const section = $derived(seg[0] ?? 'reports');
	const project = $derived(seg.length === 1 && section === 'reports' ? router.query.get('project') : null);
	const signedOut = $derived(me.error instanceof ApiError && me.error.status === 401);
	const identity = $derived(me.data ? parseActor(me.data.actor) : null);
	const newTotal = $derived((projects.data ?? []).reduce((n, p) => n + (p.counts.new ?? 0), 0));

	$effect(() => {
		if (router.path === '/') router.navigate('/reports', { replace: true });
	});
</script>

<svelte:document use:routeLinks />

<AppShell stickySidebar navLabel="Main navigation" sidebarWidth="14rem">
	{#snippet header()}
		<Heading level={1} size="md">
			<Link href="/reports" tone="inherit" underline="none">hmm</Link>
		</Heading>
		<Text tone="faint" size="sm">admin</Text>
		<div class="header-end">
			{#if identity}
				<span class="identity" title={me.data?.actor}>
					<Icon name="user" size={14} />
					<Text size="sm" tone="muted" truncate>{identity.name}</Text>
				</span>
			{/if}
			<ThemePicker box="sm" auto placement="bottom-end" />
		</div>
	{/snippet}
	{#snippet sidebar()}
		<NavItem
			href="/reports"
			label="Reports"
			icon="mail"
			active={section === 'reports' && !project}
			badge={newTotal || undefined}
			badgeTone="info"
		/>
		<NavItem href="/keys" label="API keys" icon="key-round" active={section === 'keys'} />
		<NavItem href="/webhooks" label="Webhooks" icon="send" active={section === 'webhooks'} />
		<NavItem href="/access" label="Access log" icon="list" active={section === 'access'} />
		{#if projects.data?.length}
			<NavSection label="Projects">
				{#each projects.data as p (p.project)}
					{@const open = OPEN_STATUSES.reduce((n, s) => n + (p.counts[s] ?? 0), 0)}
					<NavItem
						href="/reports?project={encodeURIComponent(p.project)}"
						label={p.project}
						icon="folder"
						active={project === p.project}
						badge={p.counts.new || undefined}
						badgeTone="info"
						title="{p.project}: {p.counts.new ?? 0} new, {plural(open, 'open report')}"
					/>
				{/each}
			</NavSection>
		{/if}
	{/snippet}
	<div class="page">
		{#if signedOut}
			<ErrorState error={me.error} />
		{:else if section === 'reports' && seg.length === 1}
			<Reports />
		{:else if section === 'reports' && seg.length === 3}
			{#key router.path}
				<Report project={seg[1] ?? ''} id={seg[2] ?? ''} />
			{/key}
		{:else if section === 'keys' && seg.length === 1}
			<Keys />
		{:else if section === 'webhooks' && seg.length === 1}
			<Webhooks />
		{:else if section === 'webhooks' && seg[1] === 'deliveries' && seg.length === 2}
			<Deliveries />
		{:else if section === 'access' && seg.length === 1}
			<AccessLog />
		{:else if router.path !== '/'}
			<NotFound />
		{/if}
	</div>
</AppShell>
<Toaster />

<style>
	.header-end {
		margin-inline-start: auto;
		display: flex;
		align-items: center;
		gap: var(--sp-3);
		min-width: 0;
	}
	.identity {
		display: inline-flex;
		align-items: center;
		gap: var(--sp-1);
		min-width: 0;
		color: var(--text-muted);
	}
	.page {
		padding: var(--sp-5) var(--sp-5) var(--sp-10);
		max-width: 100rem;
	}
	@media (max-width: 39.999rem) {
		.page {
			padding: var(--sp-4) var(--sp-3) var(--sp-10);
		}
	}
</style>
