/**
 * The custom title bar must stay usable while a modal dialog is open (design §4.5 / §7): the
 * dialog overlay starts below it (`index.css`), it re-enables pointer events that Radix turns
 * off on `<body>`, and clicking it does not dismiss the dialog — moving, minimizing or
 * maximizing the window is not an answer to the dialog.
 */

/** Marks the title bar's root element. */
export const TITLEBAR_ATTRIBUTE = 'data-app-titlebar';

/** Whether a DOM event target is inside the title bar. */
export function isInTitlebar(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(`[${TITLEBAR_ATTRIBUTE}]`) !== null;
}

/**
 * For Radix `onInteractOutside`: keeps the layer open when the interaction is with the title
 * bar. Radix dispatches these events on the original target.
 */
export function keepOpenOnTitlebar(event: Event): void {
  if (isInTitlebar(event.target)) {
    event.preventDefault();
  }
}
