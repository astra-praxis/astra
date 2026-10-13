import { useEffect, useState } from 'react';
import { createLatestValue } from './latestValue';

/** Opt-in display throttle only; the store and terminal content remain lossless/immediate. */
export function useStreamingText(content: string, streaming: boolean) {
  const [snapshot, setSnapshot] = useState(content);
  const [updates] = useState(() => createLatestValue(setSnapshot));
  useEffect(() => {
    if (streaming) updates.push(content);
    else {
      updates.dispose();
      setSnapshot(content);
    }
  }, [content, streaming, updates]);
  useEffect(() => () => updates.dispose(), [updates]);
  return streaming ? snapshot : content;
}
