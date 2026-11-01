import type { Terminal } from "@xterm/xterm";

import { keySequence } from "./key-encoding";

const LONG_PRESS_MS = 500;
const MAX_TAP_MOVEMENT_PX = 10;
const FRICTION_MS = 325;
const MIN_SCROLL_SPEED = 0.02;

/**
 * One finger on the terminal, decided by what it does first:
 * - it moves: the terminal scrolls, and a flick keeps going and slows down. xterm.js 6.0 does not
 *   scroll on touch (xtermjs/xterm.js#5489); 6.1 fixes it, and then this part goes.
 * - it stays down: a long press selects the word under it, a drag extends the selection, and
 *   lifting calls `onLongPress` with the place for the menu: the row of the finger, at the finger.
 * - it lifts at once: a tap. On the line being typed, the cursor moves to the tap.
 */
export function handleTouchGestures(
  xterm: Terminal,
  element: HTMLElement,
  onLongPress: (menuAnchor: DOMRect) => void,
) {
  let gesture: "undecided" | "scrolling" | "selecting" | undefined;
  let startX = 0;
  let startY = 0;
  let lastY = 0;
  let lastTime = 0;
  let scrollSpeed = 0;
  let unscrolledPixels = 0;
  let momentumFrame = 0;
  let longPressTimer: ReturnType<typeof setTimeout> | undefined;
  let anchor = { start: 0, end: 0 };

  const cellAt = (x: number, y: number) => {
    const screen = element.querySelector(".xterm-screen") ?? element;
    const rect = screen.getBoundingClientRect();
    const column = Math.floor((x - rect.left) / (rect.width / xterm.cols));
    const row = Math.floor((y - rect.top) / (rect.height / xterm.rows));
    return {
      column: Math.min(Math.max(column, 0), xterm.cols - 1),
      row:
        Math.min(Math.max(row, 0), xterm.rows - 1) +
        xterm.buffer.active.viewportY,
    };
  };

  // Selections are ranges of cells counted row by row, which is how `select` takes them.
  const select = (start: number, end: number) => {
    if (end <= start) xterm.clearSelection();
    else
      xterm.select(
        start % xterm.cols,
        Math.floor(start / xterm.cols),
        end - start,
      );
  };

  const wordAt = ({ column, row }: { column: number; row: number }) => {
    const line = xterm.buffer.active.getLine(row);
    const inWord = (x: number) => {
      const cell = line?.getCell(x);
      return !!cell && (cell.getWidth() === 0 || cell.getChars().trim() !== "");
    };
    const cellIndex = row * xterm.cols + column;
    if (!inWord(column)) return { start: cellIndex, end: cellIndex };

    let start = column;
    while (start > 0 && inWord(start - 1)) start--;
    let end = column;
    while (end < xterm.cols - 1 && inWord(end + 1)) end++;
    return { start: row * xterm.cols + start, end: row * xterm.cols + end + 1 };
  };

  // The shell edits its line with the arrow keys, as xterm.js's `altClickMovesCursor` does. Only on
  // the line at the cursor, wrapped rows included, and only when no program reads the mouse.
  const moveCursorTo = ({ column, row }: { column: number; row: number }) => {
    const buffer = xterm.buffer.active;
    if (buffer.type !== "normal" || xterm.modes.mouseTrackingMode !== "none")
      return;
    const cursorRow = buffer.baseY + buffer.cursorY;
    const [top, bottom] = row < cursorRow ? [row, cursorRow] : [cursorRow, row];
    for (let wrappedRow = top + 1; wrappedRow <= bottom; wrappedRow++)
      if (!buffer.getLine(wrappedRow)?.isWrapped) return;

    const cellsToMove =
      (row - cursorRow) * xterm.cols + (column - buffer.cursorX);
    if (cellsToMove === 0) return;
    const arrowKey = keySequence(
      cellsToMove > 0 ? "ArrowRight" : "ArrowLeft",
      xterm.modes.applicationCursorKeysMode,
    );
    xterm.input(arrowKey.repeat(Math.abs(cellsToMove)));
  };

  const scrollBy = (pixels: number, x: number, y: number) => {
    const lineHeight = element.clientHeight / xterm.rows;
    unscrolledPixels += pixels;
    const lines = Math.trunc(unscrolledPixels / lineHeight);
    if (lines === 0) return;
    unscrolledPixels -= lines * lineHeight;

    if (xterm.buffer.active.type === "normal") {
      xterm.scrollLines(lines);
      return;
    }
    // A full-screen program gets wheel events, which xterm.js turns into arrow keys or mouse
    // reports. One line per event: xterm.js counts line deltas exactly, and drops part of pixel ones.
    const screen = element.querySelector(".xterm-screen");
    for (let line = 0; line < Math.abs(lines); line++)
      screen?.dispatchEvent(
        new WheelEvent("wheel", {
          deltaY: Math.sign(lines),
          deltaMode: WheelEvent.DOM_DELTA_LINE,
          clientX: x,
          clientY: y,
          bubbles: true,
          cancelable: true,
        }),
      );
  };

  const onTouchStart = (event: TouchEvent) => {
    // A finger on the terminal belongs to these gestures. The right-click menu around the terminal
    // also opens on a long press, and must not see it.
    event.stopPropagation();
    cancelAnimationFrame(momentumFrame);
    clearTimeout(longPressTimer);
    const touch = event.touches.item(0);
    if (!touch || event.touches.length !== 1) {
      gesture = undefined;
      return;
    }

    gesture = "undecided";
    startX = touch.clientX;
    startY = touch.clientY;
    lastY = touch.clientY;
    lastTime = event.timeStamp;
    scrollSpeed = 0;
    unscrolledPixels = 0;
    longPressTimer = setTimeout(() => {
      gesture = "selecting";
      anchor = wordAt(cellAt(startX, startY));
      select(anchor.start, anchor.end);
    }, LONG_PRESS_MS);
  };

  const onTouchMove = (event: TouchEvent) => {
    const touch = event.touches.item(0);
    if (!touch || event.touches.length !== 1 || !gesture) return;
    const { clientX, clientY } = touch;

    if (
      gesture === "undecided" &&
      Math.hypot(clientX - startX, clientY - startY) > MAX_TAP_MOVEMENT_PX
    ) {
      clearTimeout(longPressTimer);
      gesture = "scrolling";
    }
    if (gesture === "undecided") return;
    event.preventDefault();

    if (gesture === "selecting") {
      const { column, row } = cellAt(clientX, clientY);
      const cellIndex = row * xterm.cols + column;
      select(
        Math.min(anchor.start, cellIndex),
        Math.max(anchor.end, cellIndex + 1),
      );
      return;
    }

    const pixels = lastY - clientY;
    const elapsed = Math.max(event.timeStamp - lastTime, 1);
    scrollSpeed = 0.8 * (pixels / elapsed) + 0.2 * scrollSpeed;
    lastY = clientY;
    lastTime = event.timeStamp;
    scrollBy(pixels, clientX, clientY);
  };

  const onTouchEnd = (event: TouchEvent) => {
    clearTimeout(longPressTimer);
    const touch = event.changedTouches.item(0);
    if (!touch || event.touches.length !== 0) return;
    const finishedGesture = gesture;
    gesture = undefined;

    if (finishedGesture === "selecting") {
      // No click and no focus change after a long press; the menu takes over.
      event.preventDefault();
      // The menu goes above the row of the finger, so that it does not cover the selected text.
      const screen = element.querySelector(".xterm-screen") ?? element;
      const rect = screen.getBoundingClientRect();
      const rowHeight = rect.height / xterm.rows;
      const row = Math.min(
        Math.max(Math.floor((touch.clientY - rect.top) / rowHeight), 0),
        xterm.rows - 1,
      );
      onLongPress(
        new DOMRect(touch.clientX, rect.top + row * rowHeight, 0, rowHeight),
      );
      return;
    }

    if (finishedGesture === "undecided") {
      if (xterm.hasSelection()) xterm.clearSelection();
      else moveCursorTo(cellAt(touch.clientX, touch.clientY));
      return;
    }

    if (
      finishedGesture !== "scrolling" ||
      Math.abs(scrollSpeed) < MIN_SCROLL_SPEED
    )
      return;
    const { clientX, clientY } = touch;
    let previousFrameTime = performance.now();
    const continueScroll = (now: number) => {
      const elapsed = now - previousFrameTime;
      previousFrameTime = now;
      scrollSpeed *= Math.exp(-elapsed / FRICTION_MS);
      if (Math.abs(scrollSpeed) < MIN_SCROLL_SPEED) return;
      scrollBy(scrollSpeed * elapsed, clientX, clientY);
      momentumFrame = requestAnimationFrame(continueScroll);
    };
    momentumFrame = requestAnimationFrame(continueScroll);
  };

  // Android also fires it for a long press, which the touch handlers already turned into the menu.
  const onContextMenu = (event: Event) => {
    if (!gesture) return;
    event.preventDefault();
    event.stopPropagation();
  };

  element.addEventListener("touchstart", onTouchStart, { passive: true });
  element.addEventListener("touchmove", onTouchMove, { passive: false });
  element.addEventListener("touchend", onTouchEnd, { passive: false });
  element.addEventListener("touchcancel", onTouchEnd, { passive: false });
  element.addEventListener("contextmenu", onContextMenu);

  return () => {
    cancelAnimationFrame(momentumFrame);
    clearTimeout(longPressTimer);
    element.removeEventListener("touchstart", onTouchStart);
    element.removeEventListener("touchmove", onTouchMove);
    element.removeEventListener("touchend", onTouchEnd);
    element.removeEventListener("touchcancel", onTouchEnd);
    element.removeEventListener("contextmenu", onContextMenu);
  };
}
