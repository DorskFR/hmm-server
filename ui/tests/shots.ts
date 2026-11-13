import { mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { chromium, type Page } from '@playwright/test';

const OUT = fileURLToPath(new URL('../../.qa/ui-shots/', import.meta.url));
const BASE = process.env.BASE_URL ?? 'http://127.0.0.1:4189';
const THEMES = [
	{ id: 'dark', mode: 'dark' },
	{ id: 'light', mode: 'light' },
] as const;
const SIZES = [
	{ name: 'desktop', width: 1440, height: 900 },
	{ name: 'phone', width: 390, height: 844 },
] as const;

type Step = { name: string; path: string; act?: (page: Page) => Promise<void> };

const STEPS: Step[] = [
	{ name: 'reports', path: '/reports' },
	{ name: 'reports-project', path: '/reports?project=cctui&status=all' },
	{ name: 'reports-empty', path: '/reports?q=zzzz' },
	{ name: 'report', path: '/reports/cctui/r_8f2a91' },
	{
		name: 'report-zoom',
		path: '/reports/cctui/r_8f2a91',
		act: (page) => page.getByRole('button', { name: /screenshot 1 of 2/i }).click(),
	},
	{
		name: 'report-envelope',
		path: '/reports/cctui/r_8f2a91',
		act: async (page) => {
			await page.getByRole('button', { name: 'Envelope JSON' }).click();
			await page.getByRole('button', { name: 'Envelope JSON' }).scrollIntoViewIfNeeded();
		},
	},
	{ name: 'keys', path: '/keys' },
	{
		name: 'keys-new',
		path: '/keys',
		act: (page) => page.getByRole('button', { name: 'New key' }).click(),
	},
	{
		name: 'keys-token',
		path: '/keys',
		act: async (page) => {
			await page.getByRole('button', { name: 'New key' }).click();
			await page.getByLabel('Name').fill('shots-agent');
			await page.locator('label', { hasText: 'All projects, including future ones' }).click();
			await page.getByRole('button', { name: 'Create key' }).click();
			await page.getByTestId('secret-value').waitFor();
		},
	},
	{ name: 'webhooks', path: '/webhooks' },
	{
		name: 'webhooks-new',
		path: '/webhooks',
		act: (page) => page.getByRole('button', { name: 'New webhook' }).click(),
	},
	{ name: 'deliveries', path: '/webhooks/deliveries' },
	{ name: 'access', path: '/access' },
	{ name: 'unauthorized', path: '/reports' },
];

mkdirSync(OUT, { recursive: true });
const only = process.argv[2];
const browser = await chromium.launch();
for (const size of SIZES) {
	for (const theme of THEMES) {
		const context = await browser.newContext({
			viewport: { width: size.width, height: size.height },
			deviceScaleFactor: size.name === 'phone' ? 2 : 1,
			hasTouch: size.name === 'phone',
			isMobile: size.name === 'phone',
		});
		await context.addInitScript(
			(pref) => {
				localStorage.setItem('tsumikit-theme', JSON.stringify(pref));
			},
			{ mode: theme.mode, light: 'light', dark: 'dark' },
		);
		for (const step of STEPS) {
			if (only && !step.name.startsWith(only)) continue;
			const page = await context.newPage();
			await page.request.post(`${BASE}/__mock/reset`);
			if (step.name === 'unauthorized') {
				await page.route('**/v1/admin/**', (route) =>
					route.fulfill({ status: 401, json: { error: 'unauthorized' } }),
				);
			}
			await page.goto(BASE + step.path);
			await page.waitForLoadState('networkidle');
			await step.act?.(page);
			await page.waitForTimeout(700);
			await page.screenshot({
				path: `${OUT}${step.name}.${size.name}.${theme.id}.png`,
				fullPage: size.name === 'desktop' && !step.name.includes('-'),
			});
			await page.close();
		}
		await context.close();
	}
}
await browser.close();
