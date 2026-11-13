<script lang="ts">
	import { Button, Callout, CopyButton, Field, Input, Modal, Stack } from '@dorsk/tsumikit';

	let {
		title,
		label,
		value,
		warning,
		onclose,
	}: { title: string; label: string; value: string; warning: string; onclose: () => void } = $props();
</script>

<Modal {title} {onclose} size="md">
	{#snippet body()}
		<Stack gap="var(--sp-4)">
			<Callout tone="warn">{warning}</Callout>
			<Field {label}>
				<Stack gap="var(--sp-2)">
					<Input
						readonly
						mono
						{value}
						data-testid="secret-value"
						onfocus={(e) => e.currentTarget.select()}
					/>
					<div>
						<CopyButton text={value} label="Copy" copiedLabel="Copied" showLabel />
					</div>
				</Stack>
			</Field>
		</Stack>
	{/snippet}
	{#snippet footer()}
		<Button variant="primary" onclick={onclose}>Done</Button>
	{/snippet}
</Modal>
