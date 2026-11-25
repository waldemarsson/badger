// Lets node:test import app modules that import each other through `#lib/x.js`: the app
// resolves those through Vite, where `x.js` is really `x.ts`. Import this first.
import { registerHooks } from 'node:module';

registerHooks({
	resolve(specifier, context, nextResolve) {
		if (!specifier.startsWith('#lib/') || !specifier.endsWith('.js')) return nextResolve(specifier, context);
		try {
			return nextResolve(specifier, context);
		} catch {
			return nextResolve(specifier.slice(0, -'.js'.length) + '.ts', context);
		}
	}
});
