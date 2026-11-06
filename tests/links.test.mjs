import assert from 'node:assert/strict';
import test from 'node:test';

const { isOpenableUrl } = await import('../src/lib/links.ts');

test('opens http, https and mailto in any case', () => {
	for (const href of ['http://a.se', 'https://a.se/x?y=1#z', 'HTTPS://A.SE', 'mailto:me@a.se', 'MailTo:me@a.se']) {
		assert.equal(isOpenableUrl(href), true, href);
	}
});

test('refuses every other link', () => {
	for (const href of [
		'',
		'javascript:alert(1)',
		'JavaScript:alert(1)',
		'data:text/html,x',
		'file:///etc/passwd',
		'vbscript:x',
		'tauri://localhost/notes',
		'foo.md',
		'#x',
		'//evil.com',
		':https'
	]) {
		assert.equal(isOpenableUrl(href), false, href);
	}
});
