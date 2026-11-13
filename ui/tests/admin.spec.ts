import { expect, type Page, test } from '@playwright/test';

test.beforeEach(async ({ request }) => {
	await request.post('/__mock/reset');
});

const rows = (page: Page) => page.locator('tbody tr');
const checkbox = (page: Page, label: string) =>
	page.getByRole('dialog').locator('label', { hasText: label });

test('reports: lists open reports and filters by status, project, kind and search', async ({
	page,
}) => {
	await page.goto('/');
	await expect(page).toHaveURL(/\/reports$/);
	await expect(page.getByRole('heading', { name: 'All projects' })).toBeVisible();
	await expect(rows(page)).toHaveCount(19);
	await expect(rows(page).first()).toContainText('The session list jumps back to the top');
	await expect(rows(page).first()).toContainText('cctui-agent');
	await expect(page.getByText('macbook')).toBeVisible();

	await page.getByRole('link', { name: /^New/ }).click();
	await expect(page).toHaveURL(/status=new/);
	await expect(rows(page)).toHaveCount(4);

	await page.getByRole('link', { name: /^All/ }).click();
	await expect(rows(page)).toHaveCount(30);
	await page.getByRole('button', { name: 'Load more reports' }).click();
	await expect(rows(page)).toHaveCount(60);

	await page
		.getByRole('complementary')
		.getByRole('link', { name: /tsumikit/ })
		.click();
	await expect(page).toHaveURL(/project=tsumikit/);
	await expect(page.getByRole('heading', { name: 'tsumikit' })).toBeVisible();
	await expect(rows(page)).toHaveCount(5);

	await page.getByLabel('Kind').selectOption('error');
	await expect(page.getByText('No report matches these filters')).toBeVisible();
	await page.getByRole('link', { name: 'Clear filters' }).click();
	await expect(rows(page)).toHaveCount(19);

	await page.getByLabel('Search reports').fill('heartbeat');
	await expect(page).toHaveURL(/q=heartbeat/);
	await expect(rows(page)).toHaveCount(2);
	for (const row of await rows(page).all())
		await expect(row).toContainText('Worker heartbeat missed');
});

test('report: change status, add a note, release the claim; the timeline follows', async ({
	page,
}) => {
	await page.goto('/reports');
	await page
		.getByRole('link', { name: /The session list jumps back/ })
		.first()
		.click();
	await expect(page).toHaveURL(/\/reports\/cctui\/r_8f2a91$/);
	await expect(page.getByRole('heading', { name: /Feedback/ })).toBeVisible();
	await expect(page.getByText('Remote address')).toHaveCount(0);

	const timeline = page.getByRole('list', { name: 'Activity' });
	await expect(timeline.getByRole('listitem')).toHaveCount(3);
	await expect(timeline).toContainText('Report received with 2 screenshots');

	await page.getByLabel('Status', { exact: true }).selectOption('triaged');
	await expect(timeline.getByRole('listitem')).toHaveCount(4);
	await expect(timeline).toContainText('You changed the status from new to triaged');
	await expect(page.getByLabel('Status', { exact: true })).toHaveValue('triaged');

	await page.getByRole('textbox', { name: 'Note' }).fill('Reproduced on a phone, handing over.');
	await page.getByRole('button', { name: 'Add note' }).click();
	await expect(timeline.getByRole('listitem')).toHaveCount(5);
	await expect(timeline).toContainText('Reproduced on a phone, handing over.');
	await expect(page.getByRole('textbox', { name: 'Note' })).toHaveValue('');

	await page.getByRole('textbox', { name: 'Ticket' }).fill('CCT-900');
	await page.getByRole('textbox', { name: 'Ticket' }).press('Enter');
	await expect(timeline).toContainText('You linked ticket CCT-900');

	await page.getByRole('button', { name: 'Release claim' }).click();
	await expect(page.getByText('Nobody holds this report.')).toBeVisible();
	await expect(timeline).toContainText('You released the claim');

	await page.getByRole('button', { name: /screenshot 1 of 2/i }).click();
	const dialog = page.getByRole('dialog');
	await expect(dialog.getByRole('img', { name: 'Screenshot 1 of 2' })).toBeVisible();
	await dialog.getByRole('button', { name: 'Next' }).click();
	await expect(dialog.getByRole('img', { name: 'Screenshot 2 of 2' })).toBeVisible();

	await page.reload();
	await expect(page.getByLabel('Status', { exact: true })).toHaveValue('triaged');
	await expect(page.getByRole('list', { name: 'Activity' }).getByRole('listitem')).toHaveCount(7);
});

test('keys: create shows the token once, a duplicate name is refused, revoke asks first', async ({
	page,
}) => {
	await page.goto('/keys');
	await expect(rows(page)).toHaveCount(5);

	await page.getByRole('button', { name: 'New key' }).click();
	const create = page.getByRole('button', { name: 'Create key' });
	await expect(create).toBeDisabled();
	await page.getByLabel('Name').fill('nightly-agent');
	await checkbox(page, 'cctui').click();
	await checkbox(page, 'Triage').click();
	await page.getByLabel('Expires').selectOption('90');
	await create.click();

	const secret = page.getByTestId('secret-value');
	await expect(secret).toHaveValue(/^hmm_[A-Za-z0-9]{8}_[A-Za-z0-9]{32}$/);
	const token = await secret.inputValue();
	await page.getByRole('button', { name: 'Done' }).click();
	await expect(secret).toHaveCount(0);
	await expect(page.locator('body')).not.toContainText(token);

	const row = rows(page).filter({ hasText: 'nightly-agent' });
	await expect(row).toContainText('reports:triage');
	await expect(row).toContainText(token.slice(0, 12));
	await expect(rows(page)).toHaveCount(6);

	await page.getByRole('button', { name: 'New key' }).click();
	await page.getByLabel('Name').fill('nightly-agent');
	await checkbox(page, 'All projects').click();
	await create.click();
	await expect(page.getByRole('alert')).toContainText('already exists');
	await page.getByRole('button', { name: 'Cancel' }).click();

	await row.hover();
	await row.getByRole('button', { name: /more/i }).click();
	await page.getByRole('menuitem', { name: 'Revoke' }).click();
	await expect(page.getByRole('dialog')).toContainText('Revoke nightly-agent?');
	await page.getByRole('button', { name: 'Revoke key' }).click();
	await expect(row).toContainText('Revoked');

	await row.hover();
	await row.getByRole('button', { name: 'Access log of nightly-agent' }).click();
	await expect(page).toHaveURL(/\/access\?key=/);
	await expect(page.getByText('nightly-agent has made no request')).toBeVisible();
});

test('webhooks: create shows the secret once, ping lands in deliveries, toggle and delete', async ({
	page,
}) => {
	await page.goto('/webhooks');
	await expect(rows(page)).toHaveCount(3);

	await page.getByRole('button', { name: 'New webhook' }).click();
	await page.getByLabel('Name').fill('ops-pager');
	await page.getByLabel('Project', { exact: true }).selectOption('tsumikit');
	await page.getByLabel('URL').fill('http://pager.example.com/hmm');
	await page.getByRole('button', { name: 'Create webhook' }).click();
	await expect(page.getByRole('alert')).toContainText('url must be an https URL');
	await page.getByLabel('URL').fill('https://pager.example.com/hmm');
	await checkbox(page, 'Status changed').click();
	await page.getByRole('button', { name: 'Create webhook' }).click();

	await expect(page.getByTestId('secret-value')).toHaveValue(/^whsec_/);
	await page.getByRole('button', { name: 'Done' }).click();
	const row = rows(page).filter({ hasText: 'ops-pager' });
	await expect(row).toContainText('report.status_changed');
	await expect(row).toContainText('Never');

	await row.hover();
	await row.getByRole('button', { name: 'Ping ops-pager' }).click();
	await expect(page.getByText('Ping queued for ops-pager')).toBeVisible();
	await page.getByRole('button', { name: 'View deliveries' }).click();
	await expect(page).toHaveURL(/\/webhooks\/deliveries\?webhook=/);
	await expect(rows(page)).toHaveCount(1);
	await expect(rows(page).first()).toContainText('ping');
	await page.getByRole('button', { name: 'Refresh' }).click();
	await expect(rows(page).first()).toContainText('Delivered');
	await expect(rows(page).first()).toContainText('HTTP 200');

	await rows(page).first().hover();
	await rows(page).first().getByRole('button', { name: 'Redeliver' }).click();
	await expect(rows(page).first()).toContainText('Pending');

	await page.getByRole('tab', { name: 'Endpoints' }).click();
	await expect(row.getByRole('switch')).toBeChecked();
	await row.getByRole('switch').click();
	await expect(page.getByText('Webhook ops-pager disabled')).toBeVisible();
	await expect(row.getByRole('switch')).not.toBeChecked();

	await row.hover();
	await row.getByRole('button', { name: /more/i }).click();
	await page.getByRole('menuitem', { name: 'Edit' }).click();
	await page.getByLabel('URL').fill('https://pager.example.com/v2/hmm');
	await page.getByRole('button', { name: 'Save changes' }).click();
	await expect(row).toContainText('https://pager.example.com/v2/hmm');

	await row.hover();
	await row.getByRole('button', { name: /more/i }).click();
	await page.getByRole('menuitem', { name: 'Delete' }).click();
	await page.getByRole('button', { name: 'Delete webhook' }).click();
	await expect(rows(page)).toHaveCount(3);
});

test('access log: filters by key and loads more', async ({ page }) => {
	await page.goto('/access');
	await expect(rows(page)).toHaveCount(50);
	await page.getByRole('button', { name: 'Load older requests' }).click();
	await expect(rows(page)).toHaveCount(100);

	await page.getByLabel('Key').selectOption({ label: 'tsumikit-triage' });
	await expect(page).toHaveURL(/key=/);
	await expect(rows(page)).toHaveCount(40);
	await expect(page.locator('tbody')).not.toContainText('cctui-agent');
});

test('a missing device certificate is explained', async ({ page }) => {
	await page.route('**/v1/admin/**', (route) =>
		route.fulfill({ status: 401, json: { error: 'unauthorized' } }),
	);
	await page.goto('/reports');
	await expect(page.getByText('This device is not recognised')).toBeVisible();
	await expect(page.getByText(/client certificate/)).toBeVisible();
});

test('an unknown address and an unknown report say so', async ({ page }) => {
	await page.goto('/nope');
	await expect(page.getByText('There is nothing at this address')).toBeVisible();
	await page.goto('/reports/cctui/r_missing');
	await expect(page.getByText('There is nothing at this address')).toBeVisible();
});

test.describe('phone', () => {
	test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });

	test('every page fits 390px and the report flow works', async ({ page }) => {
		const fits = async () => {
			const overflow = await page.evaluate(
				() => document.documentElement.scrollWidth - document.documentElement.clientWidth,
			);
			expect(overflow).toBeLessThanOrEqual(0);
		};

		await page.goto('/reports');
		await expect(rows(page)).toHaveCount(19);
		await fits();

		await page.getByRole('button', { name: /menu|navigation/i }).click();
		await page.getByRole('complementary').getByRole('link', { name: /cctui/ }).click();
		await expect(page).toHaveURL(/project=cctui/);
		await expect(page.getByRole('heading', { name: 'cctui' })).toBeVisible();
		await fits();

		await page
			.getByRole('link', { name: /The session list jumps back/ })
			.first()
			.click();
		await expect(page.getByRole('list', { name: 'Activity' })).toBeVisible();
		await fits();
		await page.getByLabel('Status', { exact: true }).selectOption('resolved');
		await expect(page.getByRole('list', { name: 'Activity' })).toContainText(
			'You changed the status from new to resolved',
		);

		for (const path of ['/keys', '/webhooks', '/webhooks/deliveries', '/access']) {
			await page.goto(path);
			await expect(rows(page).first()).toBeVisible();
			await fits();
		}
	});
});
