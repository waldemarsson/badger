// Closing a modal by clicking its backdrop. A plain `onclick` on the backdrop also fires
// when the press starts inside the dialog and is released outside (selecting text, then
// letting go past the edge): `click` goes to the common ancestor. Only a press that both
// starts and ends on the backdrop closes.

interface BackdropEvent {
	target: EventTarget | null;
	currentTarget: EventTarget | null;
}

/** Spread the result on the backdrop element: `<div {...backdrop}>`. */
export function backdropDismiss(onclose: () => void): {
	onpointerdown: (e: BackdropEvent) => void;
	onclick: (e: BackdropEvent) => void;
} {
	let pressed = false;
	return {
		onpointerdown: (e) => {
			pressed = e.target === e.currentTarget;
		},
		onclick: (e) => {
			const close = pressed && e.target === e.currentTarget;
			pressed = false;
			if (close) onclose();
		}
	};
}
