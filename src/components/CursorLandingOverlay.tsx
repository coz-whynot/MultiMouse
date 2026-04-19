import { useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';

interface Ripple {
  id: number;
  x: number;
  y: number;
}

/**
 * Shown on the RECEIVER side when `focus-acquired` fires. Expanding
 * concentric rings give a visual "the cursor just landed here" cue.
 *
 * Positioned absolute inset-0 + pointer-events-none so it never blocks
 * underlying UI. Each ripple auto-removes itself after the animation.
 */
export const CursorLandingOverlay = () => {
  const [ripples, setRipples] = useState<Ripple[]>([]);
  const timers = useRef<number[]>([]);

  useEffect(() => {
    let nextId = 1;
    const spawn = () => {
      // Center of viewport; the remote cursor enters somewhere on screen
      // but we don't have its exact position on the overlay mount, so
      // fall back to the viewport center.
      const x = window.innerWidth / 2;
      const y = window.innerHeight / 2;
      const id = nextId++;
      setRipples((prev) => [...prev, { id, x, y }]);
      const t = window.setTimeout(() => {
        setRipples((prev) => prev.filter((r) => r.id !== id));
        timers.current = timers.current.filter((x) => x !== t);
      }, 900);
      timers.current.push(t);
    };

    // Rely on mount-triggered spawn — the overlay is conditionally rendered
    // when is_controlled flips true, which App.tsx does in response to
    // `focus-acquired`. Unmount on release means the next acquisition is a
    // fresh mount and ripples again. A separate listener would double-fire
    // on first mount alongside this spawn().
    spawn();

    return () => {
      for (const t of timers.current) window.clearTimeout(t);
      timers.current = [];
    };
  }, []);

  return (
    <div className="absolute inset-0 pointer-events-none z-50 overflow-hidden">
      <AnimatePresence>
        {ripples.map((r) => (
          <motion.div
            key={r.id}
            className="absolute"
            style={{ left: r.x, top: r.y }}
          >
            {[0, 0.15].map((delay, i) => (
              <motion.div
                key={i}
                className="absolute rounded-full"
                style={{
                  left: -6,
                  top: -6,
                  width: 12,
                  height: 12,
                  border: '2px solid rgba(167,139,250,0.85)',
                  boxShadow: '0 0 16px rgba(99,102,241,0.55)',
                }}
                initial={{ scale: 0.4, opacity: 0.9 }}
                animate={{ scale: 14, opacity: 0 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.8, delay, ease: 'easeOut' }}
              />
            ))}
            <motion.div
              className="absolute rounded-full"
              style={{
                left: -3,
                top: -3,
                width: 6,
                height: 6,
                background:
                  'radial-gradient(circle, rgba(255,255,255,0.95) 0%, rgba(168,85,247,0.75) 60%, rgba(99,102,241,0) 100%)',
              }}
              initial={{ scale: 0, opacity: 0.95 }}
              animate={{ scale: 6, opacity: 0 }}
              transition={{ duration: 0.8, ease: 'easeOut' }}
            />
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
};
