// kto Playwright render script
// Fetches a page with full JavaScript rendering

import { chromium } from 'playwright';

const url = process.argv[2];
const timeout = parseInt(process.argv[3] || '30000');

if (!url) {
  console.error(JSON.stringify({ error: 'URL argument required' }));
  process.exit(1);
}

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage();

try {
  await page.goto(url, { waitUntil: 'networkidle', timeout });

  const result = {
    url: page.url(),
    title: await page.title(),
    html: await page.content(),
    text: await page.evaluate(() => document.body.innerText),
  };

  console.log(JSON.stringify(result));
} catch (error) {
  console.error(JSON.stringify({ error: error.message }));
  process.exit(1);
} finally {
  await browser.close();
}
