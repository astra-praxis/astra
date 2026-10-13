import { useEffect, useRef, useState } from 'react';
import { createOutputScroll } from './outputScroll';

/** Observe rendered height too: throttled markdown, images and collapsing blocks resize later. */
export function useOutputScroll(resetKey?: string) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const [controller] = useState(() => createOutputScroll(
    () => viewportRef.current,
    (callback) => requestAnimationFrame(callback),
    (id) => cancelAnimationFrame(id),
  ));

  useEffect(() => {
    controller.resume();
    controller.schedule();
    const observer = new ResizeObserver(() => controller.schedule());
    if (contentRef.current) observer.observe(contentRef.current);
    if (viewportRef.current) observer.observe(viewportRef.current);
    return () => { observer.disconnect(); controller.dispose(); };
  }, [controller, resetKey]);

  return { viewportRef, contentRef, controller };
}
