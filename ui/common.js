// Helpers shared by the quick-paste and options windows.
'use strict';

(function () {
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args || {});
  const listen = (event, cb) => T.event.listen(event, cb);
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

  function applyAppearance(settings) {
    const root = document.documentElement;
    root.dataset.theme = settings.theme || 'system';
    root.style.setProperty('--font-size', `${settings.font_size || 13}px`);
    root.style.setProperty('--lines', String(settings.lines_per_clip || 2));
  }

  // ---- interface size ----------------------------------------------------------------------
  // Both windows are zoomed by the app ("Interface size", settings.ui_zoom, in percent).

  const ZOOM_STEPS = [75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

  /** The next interface size up (direction > 0) or down from `current`. */
  function nextZoom(current, direction) {
    if (direction > 0) return ZOOM_STEPS.find((z) => z > current) || current;
    return [...ZOOM_STEPS].reverse().find((z) => z < current) || current;
  }

  /** +1 for ⌘+ / Ctrl++ (also without Shift, on the = key), -1 for ⌘− / Ctrl+−, otherwise 0. */
  function zoomKey(e) {
    if (!(isMac ? e.metaKey : e.ctrlKey) || e.altKey) return 0;
    if (e.key === '+' || e.key === '=' || e.code === 'NumpadAdd') return 1;
    if (e.key === '-' || e.key === '_' || e.code === 'NumpadSubtract') return -1;
    return 0;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = text;
    return node;
  }

  function formatBytes(n) {
    if (!Number.isFinite(n)) return '';
    if (n < 1024) return `${n} B`;
    const units = ['KB', 'MB', 'GB'];
    let v = n / 1024;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
    return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
  }

  function timeAgo(ms) {
    if (!ms) return '';
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 45) return 'just now';
    if (s < 3600) return `${Math.round(s / 60)} min ago`;
    if (s < 86400) return `${Math.round(s / 3600)} h ago`;
    if (s < 86400 * 30) return `${Math.round(s / 86400)} d ago`;
    return new Date(ms).toLocaleDateString();
  }

  function formatDate(ms) {
    return ms ? new Date(ms).toLocaleString() : '—';
  }

  // ---- accelerators ------------------------------------------------------------------
  // Stored in the syntax of the global-hotkey crate: "Ctrl+Shift+KeyV", "Super+Backquote".

  const MODIFIER_CODES = new Set(['ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight', 'OSLeft', 'OSRight', 'CapsLock', 'Fn']);

  function acceleratorFromEvent(e) {
    if (MODIFIER_CODES.has(e.code) || !e.code) return null;
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    parts.push(e.code);
    return parts.join('+');
  }

  const KEY_NAMES = {
    Backquote: '`', Minus: '-', Equal: '=', BracketLeft: '[', BracketRight: ']', Backslash: '\\',
    Semicolon: ';', Quote: "'", Comma: ',', Period: '.', Slash: '/', Space: 'Space', Enter: 'Enter',
    Escape: 'Esc', Backspace: 'Backspace', Delete: 'Del', Insert: 'Ins', Tab: 'Tab',
    ArrowUp: '↑', ArrowDown: '↓', ArrowLeft: '←', ArrowRight: '→', PageUp: 'PgUp', PageDown: 'PgDn',
    Home: 'Home', End: 'End',
    // ISO keyboards: the extra key next to Z (Apple ISO keyboards: the § key left of 1).
    IntlBackslash: isMac ? '§' : 'ISO \\', IntlRo: 'Ro', IntlYen: '¥',
  };

  function prettyKey(code) {
    if (/^Key[A-Z]$/.test(code)) return code.slice(3);
    if (/^Digit\d$/.test(code)) return code.slice(5);
    if (/^Numpad\d$/.test(code)) return `Num ${code.slice(6)}`;
    return KEY_NAMES[code] || code;
  }

  function prettyAccelerator(acc) {
    if (!acc) return '';
    const parts = acc.split('+').filter(Boolean);
    const key = prettyKey(parts.pop() || '');
    const mods = parts.map((m) => {
      const k = m.toLowerCase();
      if (isMac) {
        return { ctrl: '⌃', control: '⌃', alt: '⌥', option: '⌥', shift: '⇧', super: '⌘', cmd: '⌘', command: '⌘', cmdorctrl: '⌘', commandorcontrol: '⌘' }[k] || m;
      }
      return { ctrl: 'Ctrl', control: 'Ctrl', alt: 'Alt', option: 'Alt', shift: 'Shift', super: 'Win', cmd: 'Win', command: 'Win', cmdorctrl: 'Ctrl', commandorcontrol: 'Ctrl' }[k] || m;
    });
    return isMac ? mods.join('') + key : [...mods, key].join('+');
  }

  /** Turn an element into a "press a shortcut" field. `onChange(accelerator|'')`. */
  function hotkeyInput(value, onChange, { placeholder = 'Not set' } = {}) {
    const box = el('div', 'hotkey-input');
    box.tabIndex = 0;
    box.setAttribute('role', 'button');
    let current = value || '';
    let recording = false;
    const render = () => {
      box.textContent = '';
      if (recording) {
        box.append(el('span', 'placeholder', 'Press a shortcut…'));
      } else if (current) {
        box.append(el('kbd', null, prettyAccelerator(current)));
      } else {
        box.append(el('span', 'placeholder', placeholder));
      }
      box.classList.toggle('recording', recording);
      box.title = recording ? 'Esc cancels, Backspace clears' : 'Click and press a key combination';
    };
    box.addEventListener('click', () => { recording = true; render(); });
    box.addEventListener('blur', () => { recording = false; render(); });
    box.addEventListener('keydown', (e) => {
      if (!recording) {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); recording = true; render(); }
        return;
      }
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') { recording = false; render(); return; }
      if ((e.key === 'Backspace' || e.key === 'Delete') && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
        current = '';
        recording = false;
        render();
        onChange('');
        return;
      }
      const acc = acceleratorFromEvent(e);
      if (!acc) return;
      current = acc;
      recording = false;
      render();
      onChange(acc);
    });
    box.setValue = (v) => { current = v || ''; recording = false; render(); };
    render();
    return box;
  }

  let toastTimer = null;
  function toast(message, kind) {
    let node = document.getElementById('toast');
    if (!node) {
      node = el('div', 'toast');
      node.id = 'toast';
      document.body.append(node);
    }
    node.textContent = message;
    node.className = `toast${kind === 'error' ? ' error' : ''}`;
    node.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { node.hidden = true; }, kind === 'error' ? 4500 : 2200);
  }

  window.Ditto = { T, invoke, listen, isMac, applyAppearance, ZOOM_STEPS, nextZoom, zoomKey, el, formatBytes, timeAgo, formatDate, acceleratorFromEvent, prettyAccelerator, hotkeyInput, toast };
})();
