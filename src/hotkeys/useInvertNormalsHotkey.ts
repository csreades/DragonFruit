import { useEffect } from 'react';
import { matchesConfiguredHotkeyDown } from './hotkeyConfig';
import { useHotkeyConfig } from './HotkeyContext';

function isTextInput(element: EventTarget | null): boolean {
  if (!element || !(element instanceof HTMLElement)) return false;
  const tag = element.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  if (element.isContentEditable) return true;
  return false;
}

export function useInvertNormalsHotkey(onToggle: () => void) {
  const { getHotkey } = useHotkeyConfig();
  const toggleKey = getHotkey('CAMERA', 'INVERT_NORMALS');

  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      if (isTextInput(e.target)) return;

      const matches = matchesConfiguredHotkeyDown(e, { key: toggleKey.key, modifier: toggleKey.modifier });
      if (matches && !e.repeat) {
        e.preventDefault();
        onToggle();
      }
    };

    window.addEventListener('keydown', down, true);
    return () => {
      window.removeEventListener('keydown', down, true);
    };
  }, [toggleKey, onToggle]);
}
