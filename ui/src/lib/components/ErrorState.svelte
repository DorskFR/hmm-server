<script lang="ts">
	import { Button, EmptyState } from '@dorsk/tsumikit';
	import { ApiError } from '../api.ts';
	import { errorText } from '../labels.ts';

	let {
		error,
		onretry,
		size = 'default',
	}: { error: unknown; onretry?: () => void; size?: 'compact' | 'default' } = $props();

	const status = $derived(error instanceof ApiError ? error.status : -1);
	const copy = $derived.by(() => {
		if (status === 401) {
			return {
				icon: 'lock' as const,
				title: 'This device is not recognised',
				description:
					'The admin site identifies you by the client certificate installed on this device, and none reached the server. Install the device certificate, or open this page from a device that has one, then reload.',
				retry: 'Reload',
			};
		}
		if (status === 403) {
			return {
				icon: 'lock' as const,
				title: 'You are not allowed to see this',
				description: 'Your identity does not have the scope this page needs.',
				retry: 'Try again',
			};
		}
		if (status === 404) {
			return {
				icon: 'search' as const,
				title: 'There is nothing at this address',
				description: 'It may have been deleted, or the link is incomplete.',
				retry: 'Try again',
			};
		}
		if (status === 0) {
			return {
				icon: 'alert-circle' as const,
				title: 'The server could not be reached',
				description: 'Check the connection, then try again. Nothing was changed.',
				retry: 'Try again',
			};
		}
		return {
			icon: 'alert-circle' as const,
			title: 'This could not be loaded',
			description: `${errorText(error)} Trying again usually works.`,
			retry: 'Try again',
		};
	});
</script>

<EmptyState tone="danger" icon={copy.icon} title={copy.title} description={copy.description} {size}>
	{#snippet action()}
		{#if status === 401}
			<Button onclick={() => window.location.reload()}>{copy.retry}</Button>
		{:else if onretry}
			<Button onclick={onretry}>{copy.retry}</Button>
		{/if}
	{/snippet}
</EmptyState>
