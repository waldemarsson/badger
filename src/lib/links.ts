// Which links the notes read view may open in the default browser. Rust already drops
// every other link when rendering; this is the second check at click time.

const OPENABLE_SCHEMES = ['http:', 'https:', 'mailto:'];

/** Is `href` an absolute `http:`, `https:` or `mailto:` URL? */
export function isOpenableUrl(href: string): boolean {
	const colon = href.indexOf(':');
	if (colon < 1) return false;
	return OPENABLE_SCHEMES.includes(href.slice(0, colon + 1).trim().toLowerCase());
}
