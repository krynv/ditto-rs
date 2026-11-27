// Ditto-RS website: download links for the visitor's system, and the live demo.
(function () {
  'use strict';

  const REPO = 'krynv/ditto-rs';
  const $ = (id) => document.getElementById(id);

  // ---- which system is this? ----------------------------------------------------------------

  const hint = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || '';
  const phone = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent) || (/Mac/i.test(hint) && navigator.maxTouchPoints > 1);
  const os = phone ? null : /Mac/i.test(hint) ? 'macos' : /Win/i.test(hint) ? 'windows' : /Linux|X11|CrOS/i.test(hint + navigator.userAgent) ? 'linux' : null;
  const OS_NAME = { macos: 'macOS', windows: 'Windows', linux: 'Linux' };
  const isMac = os === 'macos';

  // Copy is ⌘C on a Mac. (The Ditto-RS hot key is Ctrl + ` everywhere.)
  if (isMac) document.querySelectorAll('.mod-key').forEach((key) => { key.textContent = '⌘'; });

  if (os) {
    const card = document.querySelector(`.os[data-os="${os}"]`);
    card.classList.add('yours');
    const tag = document.createElement('span');
    tag.className = 'yours-tag';
    tag.textContent = 'Your system';
    card.querySelector('h3').append(' ', tag);
    card.parentElement.prepend(card);
    for (const id of ['hero-download', 'final-download']) $(id).textContent = `Download for ${OS_NAME[os]}`;
  } else if (phone) {
    $('hero-note').textContent = 'Ditto-RS is a desktop app for Windows, macOS and Linux. Open this page on your computer to download it.';
  }

  // ---- the latest release -------------------------------------------------------------------
  // Without JavaScript, or if GitHub cannot be reached, every button leads to the releases page.

  const ASSETS = { dmg: /\.dmg$/, exe: /-setup\.exe$/, msi: /\.msi$/, appimage: /\.AppImage$/, deb: /\.deb$/, rpm: /\.rpm$/ };
  const PRIMARY = { macos: 'dmg', windows: 'exe', linux: 'appimage' };
  const megabytes = (bytes) => `${(bytes / 1048576).toFixed(bytes < 10485760 ? 1 : 0)} MB`;

  function showRelease(release) {
    const found = {};
    for (const [key, pattern] of Object.entries(ASSETS)) found[key] = release.assets.find((asset) => pattern.test(asset.name));
    for (const link of document.querySelectorAll('[data-asset]')) {
      const asset = found[link.dataset.asset];
      if (!asset) continue;
      link.href = asset.browser_download_url;
      if (link.classList.contains('quiet')) link.textContent = `${link.textContent} (${megabytes(asset.size)})`;
      else link.closest('.os').querySelector('.req').append(` · ${megabytes(asset.size)}`);
    }
    const version = release.tag_name.replace(/^v/, '');
    const date = new Date(release.published_at).toLocaleDateString(undefined, { year: 'numeric', month: 'long', day: 'numeric' });
    $('download-version').textContent = `Version ${version}, released ${date}. Free, for as long as you like.`;
    const mine = os && found[PRIMARY[os]];
    if (mine) {
      for (const id of ['hero-download', 'final-download']) $(id).href = mine.browser_download_url;
      const others = Object.keys(OS_NAME).filter((name) => name !== os).map((name) => OS_NAME[name]).join(' and ');
      $('hero-note').textContent = `Version ${version} · ${megabytes(mine.size)} · Also for ${others}. No account, no tracking.`;
    }
  }

  fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: 'application/vnd.github+json' } })
    .then((response) => (response.ok ? response.json() : Promise.reject(new Error(String(response.status)))))
    .then(showRelease)
    .catch(() => {});

  // ---- the live demo ------------------------------------------------------------------------
  // demo/ui/ is the app's own quick-paste page; demo/mock.js stands in for its backend and
  // calls the functions in `window.DittoDemo` below.

  const popup = $('demo-popup');
  const note = $('demo-note');
  const toast = $('demo-toast');
  if (!popup || !note) return;

  let frame = null;
  let toastTimer = null;
  const backend = () => (frame && frame.contentWindow ? frame.contentWindow.DittoDemoBackend : null);

  function say(message) {
    toast.textContent = message;
    toast.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast.classList.remove('show'), 4200);
  }

  function focusPopup() {
    if (!frame) return;
    frame.contentWindow.focus();
    const search = frame.contentDocument && frame.contentDocument.getElementById('search');
    if (search) search.focus({ preventScroll: true });
  }

  function showPopup() {
    if (!frame) return;
    closeMenus();
    if (popup.hidden) {
      popup.hidden = false;
      if (backend()) backend().shown();
    }
    focusPopup();
  }

  function hidePopup(focusNote) {
    closeMenus();
    popup.hidden = true;
    if (focusNote) note.focus({ preventScroll: true });
  }

  function startDemo() {
    if (frame) return showPopup();
    frame = document.createElement('iframe');
    frame.title = 'Ditto-RS quick-paste window (interactive demo)';
    frame.src = 'demo/ui/index.html?theme=system';
    frame.addEventListener('load', () => {
      $('demo-start-panel').hidden = true;
      $('demo-open').hidden = false;
      focusPopup();
      say('Ditto-RS is open. Type to search, then press Enter to paste.');
    }, { once: true });
    popup.append(frame);
    popup.hidden = false;
  }

  $('demo-start').addEventListener('click', startDemo);
  $('demo-open').addEventListener('click', showPopup);

  // The hot key, as in the app.
  document.addEventListener('keydown', (e) => {
    if (!e.ctrlKey || e.code !== 'Backquote') return;
    if (frame) {
      e.preventDefault();
      showPopup();
    } else if (document.activeElement === note) {
      e.preventDefault();
      startDemo();
    }
  });

  // Copying in the note adds to the history, like copying anywhere does in the app.
  for (const type of ['copy', 'cut']) {
    note.addEventListener(type, () => {
      const text = note.value.slice(note.selectionStart, note.selectionEnd);
      if (!text.trim()) return;
      if (backend() && backend().copied(text)) say('Copied. It is at the top of Ditto-RS now: press Ctrl + ` to see it.');
      else if (!frame) say('Copied. Start the demo to see it in Ditto-RS.');
    });
  }

  // Clicking back into another app hides the window, unless it is pinned.
  note.addEventListener('focus', () => {
    if (!popup.hidden && backend() && !backend().keepOpen()) hidePopup(false);
  });

  // ---- the demo's menus ---------------------------------------------------------------------

  let menus = []; // open levels: { list, parent }
  let backdrop = null;

  const KEY_NAMES = isMac
    ? { Cmd: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧', Enter: '↩', Backspace: '⌫', Delete: '⌦', Comma: ',' }
    : { Cmd: 'Ctrl', Comma: ',', Delete: 'Del' };
  const prettyKeys = (accelerator) => accelerator.split('+').map((part) => KEY_NAMES[part] || part).join(isMac ? '' : '+');

  function closeFrom(level) {
    while (menus.length > level) {
      const menu = menus.pop();
      if (menu.parent) menu.parent.setAttribute('aria-expanded', 'false');
      menu.list.remove();
    }
  }

  function closeMenus(returnFocus) {
    if (!menus.length && !backdrop) return;
    closeFrom(0);
    if (backdrop) backdrop.remove();
    backdrop = null;
    if (returnFocus) focusPopup();
  }

  const enabledButtons = (list) => [...list.querySelectorAll('button:not([aria-disabled="true"])')];

  function openLevel(items, x, y, parent) {
    const level = menus.length;
    const list = document.createElement('ul');
    list.className = 'demo-menu';
    list.setAttribute('role', 'menu');
    for (const item of items) {
      const row = document.createElement('li');
      if (item.item === 'Separator') {
        row.className = 'rule';
        row.setAttribute('role', 'separator');
        list.append(row);
        continue;
      }
      row.setAttribute('role', 'none');
      const button = document.createElement('button');
      button.type = 'button';
      button.tabIndex = -1;
      const disabled = item.enabled === false;
      const checkable = item.checked !== undefined;
      button.setAttribute('role', checkable ? 'menuitemcheckbox' : 'menuitem');
      if (checkable) button.setAttribute('aria-checked', String(!!item.checked));
      if (disabled) button.setAttribute('aria-disabled', 'true');
      const part = (className, text) => {
        const span = document.createElement('span');
        span.className = className;
        span.textContent = text;
        if (className !== 'name') span.setAttribute('aria-hidden', 'true');
        return span;
      };
      button.append(part('tick', item.checked ? '✓' : ''), part('name', item.text.replace(/&&/g, '&')));
      if (item.items) {
        button.setAttribute('aria-haspopup', 'menu');
        button.setAttribute('aria-expanded', 'false');
        button.append(part('hint', '›'));
      } else if (item.accelerator) {
        button.append(part('hint', prettyKeys(item.accelerator)));
      }
      const openSubmenu = () => {
        closeFrom(level + 1);
        if (disabled || !item.items) return null;
        const box = button.getBoundingClientRect();
        button.setAttribute('aria-expanded', 'true');
        return openLevel(item.items, box.right - 4, box.top - 5, button);
      };
      button.addEventListener('mouseenter', () => {
        button.focus({ preventScroll: true });
        openSubmenu();
      });
      button.addEventListener('click', () => {
        if (disabled) return;
        if (item.items) {
          const submenu = openSubmenu();
          const first = submenu && enabledButtons(submenu)[0];
          if (first) first.focus({ preventScroll: true });
          return;
        }
        closeMenus(true);
        if (item.action) item.action();
      });
      row.append(button);
      list.append(row);
    }
    document.body.append(list);
    // Inside the viewport; a submenu that does not fit on the right opens on the left.
    const box = list.getBoundingClientRect();
    let left = x;
    let top = y;
    if (left + box.width > innerWidth - 8) left = parent ? parent.getBoundingClientRect().left - box.width + 4 : innerWidth - box.width - 8;
    if (top + box.height > innerHeight - 8) top = innerHeight - box.height - 8;
    list.style.left = `${Math.max(8, left)}px`;
    list.style.top = `${Math.max(8, top)}px`;
    menus.push({ list, parent: parent || null });
    return list;
  }

  document.addEventListener('keydown', (e) => {
    if (!menus.length) return;
    const found = menus.findIndex((menu) => menu.list.contains(document.activeElement));
    const level = found < 0 ? menus.length - 1 : found;
    const buttons = enabledButtons(menus[level].list);
    const at = buttons.indexOf(document.activeElement);
    const back = () => {
      const parent = menus[level].parent;
      closeFrom(level);
      if (parent) parent.focus({ preventScroll: true });
    };
    switch (e.key) {
      case 'ArrowDown': buttons[(at + 1) % buttons.length].focus(); break;
      case 'ArrowUp': buttons[(at - 1 + buttons.length) % buttons.length].focus(); break;
      case 'Home': buttons[0].focus(); break;
      case 'End': buttons[buttons.length - 1].focus(); break;
      case 'ArrowRight': if (document.activeElement.getAttribute('aria-haspopup')) document.activeElement.click(); break;
      case 'ArrowLeft': if (level > 0) back(); break;
      case 'Escape': if (level > 0) back(); else closeMenus(true); break;
      case 'Tab': closeMenus(true); break;
      default: return;
    }
    e.preventDefault();
    e.stopPropagation();
  }, true);

  // ---- what the demo's backend calls --------------------------------------------------------

  window.DittoDemo = {
    /** A clip was chosen: in the app it is pasted into the app you came from. */
    pasted({ text, picture, count, keepOpen }) {
      note.setRangeText(picture ? '[the picture you copied]' : text, note.selectionStart, note.selectionEnd, 'end');
      if (keepOpen) focusPopup();
      else hidePopup(true);
      say(count > 1 ? `Pasted ${count} clips into the note.` : 'Pasted into the note. Press Ctrl + ` to open Ditto-RS again.');
    },
    hide() {
      hidePopup(true);
    },
    note: say,
    /** A menu, at a position inside the demo window. */
    menu(items, x, y) {
      closeMenus();
      const origin = frame.getBoundingClientRect();
      backdrop = document.createElement('div');
      backdrop.className = 'demo-menu-backdrop';
      backdrop.addEventListener('pointerdown', () => closeMenus(true));
      backdrop.addEventListener('contextmenu', (e) => e.preventDefault());
      document.body.append(backdrop);
      const first = enabledButtons(openLevel(items, origin.left + x, origin.top + y, null))[0];
      if (first) first.focus({ preventScroll: true });
    },
  };
})();
