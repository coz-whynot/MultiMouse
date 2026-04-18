import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';

interface UpdateInfo {
  version: string;
  body?: string;
}

export const UpdateBanner = () => {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    checkForUpdate();
  }, []);

  const checkForUpdate = async () => {
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const result = await check();
      if (result?.available) {
        setUpdate({ version: result.version, body: result.body ?? undefined });
      }
    } catch {
      // Updater not configured or no network — silently ignore
    }
  };

  const handleInstall = async () => {
    if (!update) return;
    setInstalling(true);
    try {
      const { check } = await import('@tauri-apps/plugin-updater');
      const { relaunch } = await import('@tauri-apps/plugin-process');
      const result = await check();
      if (result?.available) {
        await result.downloadAndInstall();
        await relaunch();
      }
    } catch (e) {
      console.error('Update install failed', e);
      setInstalling(false);
    }
  };

  if (dismissed || !update) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, height: 0 }}
        animate={{ opacity: 1, height: 'auto' }}
        exit={{ opacity: 0, height: 0 }}
        className="mx-3 mt-1 mb-0"
      >
        <div className="rounded-xl p-2.5 bg-accent-500/10 border border-accent-500/20 flex items-center gap-2">
          <svg className="w-3.5 h-3.5 text-accent-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
          </svg>
          <p className="text-[10px] text-accent-300 flex-1 min-w-0">
            <span className="font-semibold">v{update.version}</span> available
          </p>
          <button
            onClick={handleInstall}
            disabled={installing}
            className="text-[10px] font-semibold text-accent-400 hover:text-white
              bg-accent-500/20 hover:bg-accent-500/40 rounded-lg px-2 py-1
              transition-all disabled:opacity-50 flex-shrink-0"
          >
            {installing ? 'Installing…' : 'Update'}
          </button>
          <button
            onClick={() => setDismissed(true)}
            className="text-accent-400/50 hover:text-accent-300 flex-shrink-0"
          >
            <svg className="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>
      </motion.div>
    </AnimatePresence>
  );
};
