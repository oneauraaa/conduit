import { AnimatePresence, motion } from "motion/react";
import { Moon, Sun } from "lucide-react";
import { useTheme } from "@/lib/theme";

/**
 * Sits at the bottom-right of the sidebar. The two glyphs cross-fade and
 * counter-rotate so the switch reads as one object turning over rather than two
 * icons swapping.
 */
export function ThemeToggle() {
  const { theme, toggle } = useTheme();
  const dark = theme === "dark";

  return (
    <button
      type="button"
      onClick={toggle}
      aria-label={dark ? "Switch to light mode" : "Switch to dark mode"}
      title={dark ? "Light mode" : "Dark mode"}
      className="relative grid size-7 place-items-center overflow-hidden rounded-lg text-[rgb(var(--text-dim))] transition-colors duration-150 hover:bg-[rgb(var(--text-faint)/0.14)] hover:text-[rgb(var(--text))]"
    >
      <AnimatePresence initial={false} mode="wait">
        <motion.span
          key={theme}
          initial={{ opacity: 0, rotate: -75, scale: 0.6 }}
          animate={{ opacity: 1, rotate: 0, scale: 1 }}
          exit={{ opacity: 0, rotate: 75, scale: 0.6 }}
          transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
          className="absolute grid place-items-center"
        >
          {dark ? <Moon size={15} strokeWidth={2} /> : <Sun size={15} strokeWidth={2} />}
        </motion.span>
      </AnimatePresence>
    </button>
  );
}
