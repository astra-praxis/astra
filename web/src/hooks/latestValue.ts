/** Trailing throttle, not debounce: continuous streams still publish every interval. */
export function createLatestValue<T>(publish: (value: T) => void, delay = 100,
  schedule: (callback: () => void, delay: number) => ReturnType<typeof setTimeout> = setTimeout,
  cancel: (id: ReturnType<typeof setTimeout>) => void = clearTimeout,
) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let latest: T;
  return {
    push(value: T) {
      latest = value;
      if (timer !== undefined) return;
      timer = schedule(() => {
        timer = undefined;
        publish(latest);
      }, delay);
    },
    dispose() {
      if (timer !== undefined) cancel(timer);
      timer = undefined;
    },
  };
}
