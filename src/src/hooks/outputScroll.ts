/** One pending frame per scroll container; user intent is checked at execution time. */
export function createOutputScroll(
  getElement: () => { scrollTop: number; scrollHeight: number; clientHeight: number } | null,
  request: (callback: () => void) => number,
  cancel: (id: number) => void,
) {
  let following = true;
  let frame: number | null = null;
  let lastTop = 0;
  const stop = () => {
    if (frame !== null) cancel(frame);
    frame = null;
  };
  return {
    schedule() {
      if (!following || frame !== null) return;
      frame = request(() => {
        frame = null;
        const el = getElement();
        if (following && el) {
          el.scrollTop = el.scrollHeight;
          lastTop = el.scrollTop;
        }
      });
    },
    pause() { following = false; lastTop = getElement()?.scrollTop ?? 0; stop(); },
    resume() { following = true; lastTop = getElement()?.scrollTop ?? 0; },
    onScroll() {
      const el = getElement();
      if (el) {
        const maxTop = Math.max(0, el.scrollHeight - el.clientHeight);
        // Upward reading intent wins even within the bottom threshold. A layout
        // shrink clamping scrollTop is not an upward user gesture.
        if (el.scrollTop < lastTop - 1 && lastTop <= maxTop + 1) following = false;
        else if (el.scrollTop > lastTop && maxTop - el.scrollTop < 80) following = true;
        lastTop = el.scrollTop;
      }
      if (!following) stop();
    },
    dispose: stop,
  };
}
