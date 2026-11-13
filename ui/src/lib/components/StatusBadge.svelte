<script lang="ts">
	import { Badge } from '@dorsk/tsumikit';
	import { statusLabel, statusTone } from '../labels.ts';

	let { status, size = 'sm' }: { status: string; size?: 'xs' | 'sm' | 'md' } = $props();
</script>

<Badge tone={statusTone(status)} dot {size}>{statusLabel(status)}</Badge>
