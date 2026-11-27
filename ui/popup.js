// Quick-paste window: list, search, keyboard, menus and dialogs (Ditto's CQPasteWnd).
'use strict';

(function () {
  const D = window.Ditto;
  const { invoke, listen, isMac, el, toast } = D;
  const PAGE = 100;

  const ICONS = {
    search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>',
    dots: '<circle cx="5" cy="12" r="1.2" fill="currentColor"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/><circle cx="19" cy="12" r="1.2" fill="currentColor"/>',
    pin: '<path d="M9 4h6l-1 6 3 3v1H7v-1l3-3z"/><path d="M12 14v6"/>',
    keyboard: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10"/>',
    lock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
    folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
    chevron: '<path d="m6 9 6 6 6-6"/>',
    check: '<path d="m5 12 5 5 9-10"/>',
    close: '<path d="M6 6l12 12M18 6 6 18"/>',
    clips: '<rect x="6" y="4" width="12" height="16" rx="2"/><path d="M9 4h6v3H9z"/>',
  };

  function icon(name) {
    const holder = document.createElement('span');
    holder.innerHTML = `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">${ICONS[name]}</svg>`;
    return holder.firstChild;
  }

  const $ = (id) => document.getElementById(id);
  const listEl = $('list');
  const searchEl = $('search');
  const dialogLayer = $('dialog-layer');
  const previewEl = $('preview');

  const S = {
    settings: null,
    info: null,
    items: [],
    hasMore: false,
    loading: false,
    cursor: -1,
    anchor: -1,
    selected: new Set(),
    scope: { kind: 'main' },
    groups: [],
    query: '',
    transforms: [],
    capturePaused: false,
    targetApp: null,
    reloadSeq: 0,
    preview: { open: false, tab: null, details: null },
    thumbs: new Map(),
  };

  const MOD = isMac ? '⌘' : 'Ctrl+';
  const MODE_LABELS = { wildcard: 'Wildcard', contains: 'Contains', regex: 'Regex' };

  // ---- helpers ------------------------------------------------------------------------

  const primaryMod = (e) => (isMac ? e.metaKey : e.ctrlKey);
  const currentClip = () => (S.cursor >= 0 ? S.items[S.cursor] : null);
  const inGroup = () => S.scope.kind === 'group';
  const groupName = (id) => (S.groups.find((g) => g.id === id) || {}).name;

  function selectedIds() {
    // In list order, so multi-paste follows Ditto's order rules.
    return S.items.filter((c) => S.selected.has(c.id)).map((c) => c.id);
  }

  function selectedClips() {
    return S.items.filter((c) => S.selected.has(c.id));
  }

  async function call(cmd, args, okMessage) {
    try {
      const result = await invoke(cmd, args);
      if (okMessage) toast(okMessage);
      return result;
    } catch (e) {
      toast(String(e), 'error');
      throw e;
    }
  }

  async function saveSettings(patch) {
    const next = { ...S.settings, ...patch };
    const res = await call('save_settings', { settings: next });
    S.settings = res.settings;
    res.warnings.forEach((w) => toast(w, 'error'));
    return res.settings;
  }

  // ---- list rendering -------------------------------------------------------------------

  function displayText(desc) {
    let t = desc.replace(/\r\n?/g, '\n');
    if (!S.settings.show_leading_whitespace) t = t.replace(/^\s+/, '');
    return t;
  }

  function renderRow(clip, index) {
    const row = el('div', 'clip');
    row.dataset.index = String(index);
    row.setAttribute('role', 'option');

    const num = el('span', 'num', S.settings.show_quick_paste_numbers && index < 10 ? String((index + 1) % 10) : '');
    row.append(num);

    const body = el('div', 'body');
    if (clip.kind === 'image' && clip.has_thumbnail && S.settings.show_thumbnails) {
      const img = el('img', 'thumb');
      img.alt = clip.description;
      img.dataset.thumb = String(clip.id);
      img.draggable = false;
      const cached = S.thumbs.get(clip.id);
      if (cached) img.src = cached;
      else thumbObserver.observe(img);
      body.append(img);
    } else if (clip.kind === 'files') {
      const files = clip.description.split('\n').filter(Boolean);
      const box = el('div', 'files');
      const shown = Math.max(1, S.settings.lines_per_clip);
      files.slice(0, shown).forEach((path) => {
        const line = el('div', 'file');
        const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
        line.append(icon('file'), el('span', 'name', cut >= 0 ? path.slice(cut + 1) || path : path));
        // The folder is truncated from the left (direction: rtl); left-to-right marks keep
        // the slashes where they belong.
        const lrm = String.fromCharCode(0x200e);
        if (cut > 0) line.append(el('span', 'dir', lrm + path.slice(0, cut) + lrm));
        box.append(line);
      });
      if (files.length > shown) box.append(el('span', 'more', `+${files.length - shown} more`));
      body.append(box);
    } else {
      const text = displayText(clip.description);
      const node = el('div', 'text', text || '(blank)');
      if (!text.trim()) node.classList.add('blank');
      body.append(node);
      if (clip.kind === 'image') body.append(el('div', 'image-label', 'Image'));
    }
    row.append(body);

    const badges = el('span', 'badges');
    const sticky = inGroup() ? clip.group_sticky : clip.sticky;
    if (sticky) {
      const pin = icon('pin');
      pin.classList.add('sticky');
      badges.append(pin);
    }
    if (clip.hotkey) badges.append(icon('keyboard'));
    if (clip.never_auto_delete) badges.append(icon('lock'));
    if (!inGroup() && clip.group_id != null) badges.append(icon('folder'));
    if (badges.childNodes.length) row.append(badges);

    const parts = [];
    if (clip.source_app) parts.push(`Copied in ${clip.source_app}`);
    parts.push(D.timeAgo(clip.created_at));
    parts.push(D.formatBytes(clip.size_bytes));
    if (clip.group_id != null && groupName(clip.group_id)) parts.push(`Group: ${groupName(clip.group_id)}`);
    if (clip.hotkey) parts.push(`Hotkey: ${D.prettyAccelerator(clip.hotkey)}`);
    if (clip.quick_paste_text) parts.push(`Quick paste: ${clip.quick_paste_text}`);
    row.title = parts.join(' · ');
    return row;
  }

  function renderList() {
    listEl.textContent = '';
    const frag = document.createDocumentFragment();
    S.items.forEach((clip, i) => frag.append(renderRow(clip, i)));
    listEl.append(frag);
    updateLoadMore();
    updateSelection();
    updateEmpty();
  }

  function appendRows(start) {
    const frag = document.createDocumentFragment();
    for (let i = start; i < S.items.length; i++) frag.append(renderRow(S.items[i], i));
    const marker = listEl.querySelector('.load-more');
    listEl.insertBefore(frag, marker);
    updateLoadMore();
    updateSelection();
  }

  function updateLoadMore() {
    let marker = listEl.querySelector('.load-more');
    if (S.hasMore) {
      if (!marker) {
        marker = el('div', 'load-more', 'Loading more…');
        listEl.append(marker);
      }
    } else if (marker) {
      marker.remove();
    }
  }

  function rowAt(index) {
    return listEl.children[index] && listEl.children[index].classList.contains('clip') ? listEl.children[index] : null;
  }

  function updateSelection() {
    for (const row of listEl.querySelectorAll('.clip')) {
      const i = Number(row.dataset.index);
      const clip = S.items[i];
      const selected = !!clip && S.selected.has(clip.id);
      row.classList.toggle('selected', selected);
      row.classList.toggle('cursor', i === S.cursor);
      row.setAttribute('aria-selected', selected ? 'true' : 'false');
    }
  }

  function updateEmpty() {
    const empty = $('empty');
    if (S.items.length) {
      empty.hidden = true;
      return;
    }
    empty.textContent = '';
    if (S.query) {
      empty.append(el('strong', null, `No clips match “${S.query}”`));
      empty.append(el('span', null, `${MODE_LABELS[S.settings.search_mode]} search · Esc clears it`));
    } else if (inGroup()) {
      empty.append(el('strong', null, 'This group is empty'));
      empty.append(el('span', null, 'Right-click a clip and choose Groups ▸ Move to Group to add it here.'));
    } else {
      empty.append(el('strong', null, 'Nothing copied yet'));
      const hint = el('span');
      hint.append('Copy some text or an image and it shows up here. Press ');
      hint.append(el('kbd', null, D.prettyAccelerator(S.settings.hotkey_activate) || '—'));
      hint.append(' anywhere to open this window.');
      empty.append(hint);
    }
    empty.hidden = false;
  }

  const thumbObserver = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const img = entry.target;
      thumbObserver.unobserve(img);
      const id = Number(img.dataset.thumb);
      invoke('clip_thumbnail', { id }).then((url) => {
        if (!url) return;
        S.thumbs.set(id, url);
        img.src = url;
      }).catch(() => {});
    }
  }, { root: listEl, rootMargin: '200px' });

  // ---- data -----------------------------------------------------------------------------

  async function reload({ resetCursor = false } = {}) {
    const seq = ++S.reloadSeq;
    const keepId = resetCursor ? null : (currentClip() || {}).id;
    const keepSelected = resetCursor ? new Set() : new Set(S.selected);
    const limit = resetCursor ? PAGE : Math.max(PAGE, S.items.length);
    let res;
    try {
      res = await invoke('list_clips', { request: { scope: S.scope, query: S.query, offset: 0, limit } });
    } catch (e) {
      toast(String(e), 'error');
      return;
    }
    if (seq !== S.reloadSeq) return;
    S.items = res.items;
    S.hasMore = res.has_more;
    searchEl.classList.toggle('invalid', !!res.search_error);
    searchEl.title = res.search_error ? `Invalid pattern: ${res.search_error}` : '';
    let cursor = keepId != null ? S.items.findIndex((c) => c.id === keepId) : -1;
    if (cursor < 0) cursor = S.items.length ? Math.min(Math.max(S.cursor, 0), S.items.length - 1) : -1;
    if (resetCursor) cursor = S.items.length ? 0 : -1;
    S.selected = new Set(S.items.filter((c) => keepSelected.has(c.id)).map((c) => c.id));
    if (cursor >= 0 && !S.selected.has(S.items[cursor].id)) {
      if (!S.selected.size) S.selected.add(S.items[cursor].id);
    }
    S.cursor = cursor;
    S.anchor = cursor;
    renderList();
    if (resetCursor) listEl.scrollTop = 0;
    scrollToCursor();
    if (S.preview.open) refreshPreview();
  }

  let reloadTimer = null;
  function scheduleReload() {
    clearTimeout(reloadTimer);
    reloadTimer = setTimeout(() => reload(), 40);
  }

  async function loadMore() {
    if (S.loading || !S.hasMore) return;
    S.loading = true;
    const seq = S.reloadSeq;
    try {
      const res = await invoke('list_clips', { request: { scope: S.scope, query: S.query, offset: S.items.length, limit: PAGE } });
      if (seq !== S.reloadSeq) return;
      const start = S.items.length;
      S.items.push(...res.items);
      S.hasMore = res.has_more;
      appendRows(start);
    } catch (e) {
      toast(String(e), 'error');
    } finally {
      S.loading = false;
    }
  }

  async function loadGroups() {
    try {
      S.groups = await invoke('list_groups');
    } catch (e) {
      S.groups = [];
    }
    if (inGroup() && !S.groups.some((g) => g.id === S.scope.id)) S.scope = { kind: 'main' };
  }

  // ---- selection ------------------------------------------------------------------------

  function scrollToCursor() {
    const row = rowAt(S.cursor);
    if (row) row.scrollIntoView({ block: 'nearest' });
  }

  function moveCursor(to, { extend = false, toggle = false } = {}) {
    if (!S.items.length) return;
    const index = Math.max(0, Math.min(S.items.length - 1, to));
    const id = S.items[index].id;
    if (extend) {
      if (S.anchor < 0) S.anchor = S.cursor >= 0 ? S.cursor : index;
      const [a, b] = [Math.min(S.anchor, index), Math.max(S.anchor, index)];
      S.selected = new Set(S.items.slice(a, b + 1).map((c) => c.id));
    } else if (toggle) {
      if (S.selected.has(id) && S.selected.size > 1) S.selected.delete(id);
      else S.selected.add(id);
      S.anchor = index;
    } else {
      S.selected = new Set([id]);
      S.anchor = index;
    }
    S.cursor = index;
    updateSelection();
    scrollToCursor();
    if (index >= S.items.length - 5) loadMore();
    if (S.preview.open) refreshPreview();
  }

  function selectAll() {
    S.selected = new Set(S.items.map((c) => c.id));
    updateSelection();
  }

  function pageSize() {
    const row = listEl.querySelector('.clip');
    const h = row ? row.offsetHeight : 40;
    return Math.max(1, Math.floor(listEl.clientHeight / h) - 1);
  }

  // ---- actions --------------------------------------------------------------------------

  async function paste(opts = {}) {
    const ids = opts.ids || selectedIds();
    if (!ids.length) return;
    try {
      await invoke('paste_clips', {
        request: {
          ids,
          plain_text: !!opts.plain,
          transform: opts.transform || null,
          combine_images: opts.combine || null,
          keep_order: !!opts.keepOrder,
          from_popup: true,
        },
      });
    } catch (e) {
      toast(String(e), 'error');
    }
  }

  function pastePosition(position, plain) {
    if (position >= S.items.length) return;
    moveCursor(position);
    paste({ ids: [S.items[position].id], plain });
  }

  async function copySelection(plain) {
    const ids = selectedIds();
    if (!ids.length) return;
    await call('copy_clips', { ids, plain_text: !!plain }, ids.length > 1 ? `Copied ${ids.length} clips` : 'Copied to the clipboard');
  }

  async function deleteSelection() {
    const ids = selectedIds();
    if (!ids.length) return;
    const doDelete = async () => {
      // Keep the cursor near where it was.
      S.selected = new Set();
      await call('delete_clips', { ids });
    };
    if (!S.settings.prompt_on_delete) return doDelete();
    confirmDialog({
      title: ids.length > 1 ? `Delete ${ids.length} clips?` : 'Delete this clip?',
      message: 'This cannot be undone.',
      confirm: 'Delete',
      danger: true,
      onConfirm: doDelete,
    });
  }

  function hide() {
    invoke('hide_popup');
  }

  function clearSearch() {
    searchEl.value = '';
    applySearch();
  }

  let searchTimer = null;
  function applySearch() {
    clearTimeout(searchTimer);
    if (searchEl.value === S.query) return;
    S.query = searchEl.value;
    reload({ resetCursor: true });
  }

  async function cycleMode() {
    const order = ['wildcard', 'contains', 'regex'];
    const next = order[(order.indexOf(S.settings.search_mode) + 1) % order.length];
    await saveSettings({ search_mode: next });
    updateModeButton();
    reload({ resetCursor: true });
  }

  async function toggleKeepOpen() {
    const keep = await call('toggle_keep_open');
    S.settings.keep_open_on_blur = keep;
    updateHeader();
    toast(keep ? 'The window stays open when it loses focus' : 'The window closes when it loses focus');
  }

  async function setScope(scope) {
    S.scope = scope;
    updateHeader();
    await reload({ resetCursor: true });
  }

  // ---- header / footer --------------------------------------------------------------------

  function updateHeader() {
    $('group-name').textContent = inGroup() ? groupName(S.scope.id) || 'Group' : 'All clips';
    const groupIcon = $('group-icon');
    groupIcon.textContent = '';
    groupIcon.append(icon(inGroup() ? 'folder' : 'clips'));
    const status = $('status');
    status.textContent = '';
    if (S.capturePaused) {
      status.append(el('span', 'paused', '● Not saving copies'));
      status.title = 'Copies are not being saved. Turn "Connect To Clipboard" back on in the menu.';
    } else if (S.targetApp) {
      status.append(el('span', null, `→ ${S.targetApp}`));
      status.title = `Pastes into ${S.targetApp}`;
    } else {
      status.title = '';
    }
    $('pin-button').setAttribute('aria-pressed', S.settings.keep_open_on_blur ? 'true' : 'false');
  }

  function updateModeButton() {
    $('mode-button').textContent = MODE_LABELS[S.settings.search_mode] || 'Wildcard';
  }

  function updateBanner() {
    const banner = $('banner');
    banner.textContent = '';
    banner.hidden = true;
    if (S.info && !S.info.can_paste) {
      banner.append(el('span', null, 'Allow Accessibility access so Ditto-RS can paste for you. Until then, clips are copied and you paste with ' + (isMac ? '⌘V' : 'Ctrl+V') + '.'));
      const btn = el('button', 'btn', 'Allow…');
      btn.addEventListener('click', async () => {
        await invoke('request_paste_permission');
        S.info = await invoke('app_info');
        updateBanner();
      });
      banner.append(btn);
      banner.hidden = false;
    } else if (S.info && S.info.hotkey_problems.length) {
      banner.append(el('span', null, `Hotkey problem: ${S.info.hotkey_problems[0]}`));
      const btn = el('button', 'btn', 'Options…');
      btn.addEventListener('click', () => invoke('open_settings'));
      banner.append(btn);
      banner.hidden = false;
    }
  }

  // ---- preview (F3) -----------------------------------------------------------------------

  async function refreshPreview() {
    const clip = currentClip();
    if (!clip) {
      closePreview();
      return;
    }
    let details;
    try {
      details = await invoke('clip_details', { id: clip.id });
    } catch (e) {
      toast(String(e), 'error');
      return;
    }
    if ((currentClip() || {}).id !== clip.id) return;
    S.preview.details = details;
    const tabs = [];
    if (details.image) tabs.push(['image', 'Image']);
    if (details.files.length) tabs.push(['files', 'Files']);
    if (details.text != null) tabs.push(['text', 'Text']);
    if (details.html) tabs.push(['html', 'HTML']);
    if (details.rtf_text != null) tabs.push(['rtf', 'RTF']);
    tabs.push(['info', 'Info']);
    if (!tabs.some(([id]) => id === S.preview.tab)) S.preview.tab = tabs[0][0];
    const tabBox = $('preview-tabs');
    tabBox.textContent = '';
    for (const [id, label] of tabs) {
      const b = el('button', 'tab', label);
      b.setAttribute('role', 'tab');
      b.setAttribute('aria-selected', id === S.preview.tab ? 'true' : 'false');
      b.addEventListener('click', () => {
        S.preview.tab = id;
        refreshPreview();
      });
      tabBox.append(b);
    }
    const total = details.formats.reduce((n, f) => n + f.size, 0);
    $('preview-meta').textContent = `${D.formatBytes(total)} · ${D.timeAgo(details.summary.created_at)}`;
    renderPreviewBody(details);
    previewEl.hidden = false;
  }

  function renderPreviewBody(d) {
    const body = $('preview-body');
    body.textContent = '';
    body.classList.remove('html-mode');
    switch (S.preview.tab) {
      case 'image': {
        const img = el('img');
        img.src = d.image;
        img.alt = d.summary.description;
        body.append(img);
        if (d.image_size) body.append(el('p', 'muted', `${d.image_size[0]} × ${d.image_size[1]} px`));
        break;
      }
      case 'files':
        d.files.forEach((f) => body.append(el('div', 'file-line', f)));
        break;
      case 'text':
        body.append(el('pre', null, d.text + (d.text_truncated ? '\n…' : '')));
        break;
      case 'rtf':
        body.append(el('pre', null, d.rtf_text));
        break;
      case 'html': {
        body.classList.add('html-mode');
        const frame = el('iframe');
        frame.setAttribute('sandbox', '');
        frame.srcdoc = `<!doctype html><meta charset="utf-8"><style>body{font-family:system-ui,sans-serif;font-size:13px;margin:10px;color:#111}img{max-width:100%}</style>${d.html}`;
        body.append(frame);
        break;
      }
      default: {
        const s = d.summary;
        const table = el('table', 'info');
        const rows = [
          ['Title', s.description.split('\n')[0]],
          ['Created', D.formatDate(s.created_at)],
          ['Last used', D.formatDate(s.last_used_at)],
          ['Copied in', s.source_app || '—'],
          ['Group', s.group_id != null ? groupName(s.group_id) || s.group_id : '—'],
          ['Hot key', s.hotkey ? D.prettyAccelerator(s.hotkey) : '—'],
          ['Quick paste text', s.quick_paste_text || '—'],
          ['Sticky', s.sticky || s.group_sticky ? 'Yes' : 'No'],
          ['Never auto delete', s.never_auto_delete ? 'Yes' : 'No'],
          ['Formats', d.formats.map((f) => `${f.name} (${D.formatBytes(f.size)})`).join(', ')],
        ];
        for (const [k, v] of rows) {
          const tr = el('tr');
          tr.append(el('td', null, k), el('td', null, String(v)));
          table.append(tr);
        }
        body.append(table);
      }
    }
  }

  function togglePreview() {
    if (S.preview.open) closePreview();
    else {
      S.preview.open = true;
      refreshPreview();
    }
  }

  function closePreview() {
    S.preview.open = false;
    previewEl.hidden = true;
    searchEl.focus();
  }

  // ---- menus ------------------------------------------------------------------------------
  // The right-click, group and toolbar menus are native OS menus, as in Ditto: they can extend
  // past the small quick-paste window, and the OS handles submenus, hover and the keyboard.
  // Items are { label, shortcut?, checked?, disabled?, action? | submenu? } or '-' (separator).
  // `submenu` is an array or a function returning one; it is only built for enabled items.

  const nativeMenu = window.__TAURI__.menu;
  const { LogicalPosition } = window.__TAURI__.dpi;
  // The last menu shown. On Linux popup() returns while the menu is still open, so a menu is
  // freed when the next one opens instead of straight after popup().
  let shownMenu = null;

  /** Display shortcut ("⌘E", "Shift+Enter", "⌘⌫", "Del") → Tauri accelerator ("Cmd+E", …). */
  function accelerator(shortcut) {
    const keys = { Del: 'Delete', '⌫': 'Backspace', ',': 'Comma', Esc: 'Escape' };
    return shortcut
      .replace(/⌘/g, 'Cmd+').replace(/⌥/g, 'Alt+').replace(/⇧/g, 'Shift+').replace(/⌃/g, 'Ctrl+')
      .split('+')
      .map((part) => keys[part] || part)
      .join('+');
  }

  /** Our menu items → Tauri menu item options, so one IPC call builds the whole tree. */
  function nativeItems(items, withShortcuts) {
    const out = [];
    const isSeparator = (option) => option.item === 'Separator';
    for (const item of items) {
      if (item === '-') {
        if (out.length && !isSeparator(out[out.length - 1])) out.push({ item: 'Separator' });
        continue;
      }
      // A single & marks a mnemonic on every platform; && shows a literal &.
      const base = { text: item.label.replace(/&/g, '&&'), enabled: !item.disabled };
      if (item.submenu) {
        const children = item.disabled ? [] : typeof item.submenu === 'function' ? item.submenu() : item.submenu;
        out.push({ ...base, items: nativeItems(children, withShortcuts) });
        continue;
      }
      const option = { ...base, action: () => runMenuItem(item) };
      if (withShortcuts && item.shortcut) option.accelerator = accelerator(item.shortcut);
      if (item.checked !== undefined) option.checked = Boolean(item.checked);
      out.push(option);
    }
    if (out.length && isSeparator(out[out.length - 1])) out.pop();
    return out;
  }

  function runMenuItem(item) {
    searchEl.focus();
    if (item.action) item.action();
  }

  /** Show a menu with its top-left corner at (x, y) in window coordinates. */
  async function openMenu(items, x, y, title) {
    const list = title ? [{ label: title, disabled: true }, '-', ...items] : items;
    try {
      let menu;
      try {
        menu = await nativeMenu.Menu.new({ items: nativeItems(list, true) });
      } catch (err) {
        // A shortcut this platform can't display shouldn't cost the whole menu.
        console.warn('menu shortcuts dropped:', err);
        menu = await nativeMenu.Menu.new({ items: nativeItems(list, false) });
      }
      if (shownMenu) shownMenu.close().catch(() => {});
      shownMenu = menu;
      await invoke('context_menu_opening');
      await menu.popup(new LogicalPosition(x, y));
    } catch (err) {
      toast(`Couldn't open the menu: ${err}`, 'error');
    }
  }

  function groupTree() {
    const byParent = new Map();
    for (const g of S.groups) {
      const key = g.parent_id == null ? 'root' : g.parent_id;
      if (!byParent.has(key)) byParent.set(key, []);
      byParent.get(key).push(g);
    }
    const out = [];
    const visit = (key, depth) => (byParent.get(key) || []).forEach((g) => {
      out.push({ group: g, depth });
      visit(g.id, depth + 1);
    });
    visit('root', 0);
    return out;
  }

  function moveToGroupItems() {
    const ids = selectedIds();
    const items = groupTree().map(({ group, depth }) => ({
      label: `${'    '.repeat(depth)}${group.name}`,
      action: () => call('move_to_group', { ids, group_id: group.id }, `Moved to ${group.name}`),
    }));
    if (items.length) items.push('-');
    items.push({ label: 'Remove from group', disabled: !selectedClips().some((c) => c.group_id != null), action: () => call('move_to_group', { ids, group_id: null }) });
    items.push({ label: 'New Group…', action: () => newGroupDialog(ids) });
    return items;
  }

  function clipMenuItems() {
    const clips = selectedClips();
    const clip = currentClip();
    const one = clips.length === 1;
    const images = clips.filter((c) => c.kind === 'image').length;
    const scope = S.scope;
    const sticky = clip && (inGroup() ? clip.group_sticky : clip.sticky);
    const transforms = S.transforms.map((t) => ({ label: t.label, action: () => paste({ transform: t.id }) }));
    return [
      { label: clips.length > 1 ? `Paste ${clips.length} Clips` : 'Paste', shortcut: 'Enter', action: () => paste() },
      { label: 'Paste Plain Text Only', shortcut: 'Shift+Enter', action: () => paste({ plain: true }) },
      {
        label: 'Special Paste',
        submenu: () => [
          ...transforms.slice(0, 8),
          '-',
          ...transforms.slice(8),
          '-',
          { label: 'Multiple Images Horizontally', disabled: images < 1, action: () => paste({ combine: 'horizontal' }) },
          { label: 'Multiple Images Vertically', disabled: images < 1, action: () => paste({ combine: 'vertical' }) },
          '-',
          { label: "Paste, Don't Change Clip Order", action: () => paste({ keepOrder: true }) },
        ],
      },
      { label: 'Copy to Clipboard', shortcut: `${MOD}C`, action: () => copySelection(false) },
      '-',
      { label: 'View Full Description', shortcut: 'F3', disabled: !clip, action: togglePreview },
      { label: 'Edit Clip', shortcut: `${MOD}E`, disabled: !one || clip.kind !== 'text', action: editClipDialog },
      { label: 'New Clip', shortcut: `${MOD}N`, action: () => editorDialog(null) },
      { label: 'Save To File…', disabled: !one, action: () => call('save_clip_to_file', { id: clip.id }).then((p) => p && toast(`Saved to ${p}`)) },
      '-',
      {
        label: 'Clip Order',
        disabled: !one,
        submenu: () => [
          { label: 'Move To Top', action: () => call('move_clip', { id: clip.id, op: 'top', scope }) },
          { label: 'Move Up', action: () => call('move_clip', { id: clip.id, op: 'up', scope }) },
          { label: 'Move Down', action: () => call('move_clip', { id: clip.id, op: 'down', scope }) },
          { label: 'Move To Last', action: () => call('move_clip', { id: clip.id, op: 'last', scope }) },
          '-',
          { label: 'Make Top Sticky Clip', action: () => call('set_sticky', { id: clip.id, op: 'top', scope }) },
          { label: 'Make Last Sticky Clip', action: () => call('set_sticky', { id: clip.id, op: 'last', scope }) },
          { label: 'Replace Top Sticky Clip', action: () => call('set_sticky', { id: clip.id, op: 'replace_top', scope }) },
          { label: 'Remove Sticky Setting', disabled: !sticky, action: () => call('set_sticky', { id: clip.id, op: 'remove', scope }) },
        ],
      },
      {
        label: 'Groups',
        submenu: () => [
          { label: 'Move to Group', submenu: moveToGroupItems },
          { label: 'New Group…', shortcut: `${MOD}F7`, action: () => newGroupDialog([]) },
          { label: 'New Group From Selection…', shortcut: 'F7', action: () => newGroupDialog(selectedIds()) },
        ],
      },
      {
        label: 'Quick Properties',
        disabled: !one,
        submenu: () => [
          { label: 'Never Auto Delete', checked: clip.never_auto_delete, action: () => quickProperty(clip, { never_auto_delete: true }) },
          { label: 'Auto Delete', checked: !clip.never_auto_delete, action: () => quickProperty(clip, { never_auto_delete: false }) },
          { label: 'Remove Hot Key', disabled: !clip.hotkey, action: () => quickProperty(clip, { hotkey: null }) },
          { label: 'Remove Quick Paste', disabled: !clip.quick_paste_text, action: () => quickProperty(clip, { quick_paste_text: null }) },
        ],
      },
      { label: 'Properties…', shortcut: isMac ? '⌥Enter' : 'Alt+Enter', disabled: !one, action: propertiesDialog },
      '-',
      { label: clips.length > 1 ? `Delete ${clips.length} Clips` : 'Delete', shortcut: isMac ? '⌘⌫' : 'Del', action: deleteSelection },
    ];
  }

  async function quickProperty(clip, patch) {
    const props = {
      description: clip.description,
      quick_paste_text: clip.quick_paste_text,
      never_auto_delete: clip.never_auto_delete,
      hotkey: clip.hotkey,
      ...patch,
    };
    const problems = await call('update_clip_properties', { id: clip.id, properties: props });
    problems.forEach((p) => toast(p, 'error'));
  }

  function systemMenuItems() {
    const st = S.settings;
    const scopes = st.search_scopes;
    const setScopes = (patch) => saveSettings({ search_scopes: { ...scopes, ...patch } }).then(() => reload({ resetCursor: true }));
    const setMode = (mode) => saveSettings({ search_mode: mode }).then(() => { updateModeButton(); reload({ resetCursor: true }); });
    return [
      { label: 'Options…', shortcut: isMac ? '⌘,' : '', action: () => invoke('open_settings') },
      '-',
      {
        label: 'Search Options',
        submenu: () => [
          { label: 'Search Description', checked: scopes.description, action: () => setScopes({ description: !scopes.description }) },
          { label: 'Search Full Text', checked: scopes.full_text, action: () => setScopes({ full_text: !scopes.full_text }) },
          { label: 'Search Quick Paste', checked: scopes.quick_paste, action: () => setScopes({ quick_paste: !scopes.quick_paste }) },
          '-',
          { label: 'Contains Text Search', checked: st.search_mode === 'contains', action: () => setMode('contains') },
          { label: 'Regular Expression Search', checked: st.search_mode === 'regex', action: () => setMode('regex') },
          { label: 'Wildcard Search', checked: st.search_mode === 'wildcard', action: () => setMode('wildcard') },
        ],
      },
      {
        label: 'Quick Options',
        submenu: () => [
          { label: 'Lines Per Clip', submenu: () => [1, 2, 3, 4, 5].map((n) => ({ label: String(n), checked: st.lines_per_clip === n, action: () => saveSettings({ lines_per_clip: n }) })) },
          {
            label: 'Positioning',
            submenu: () => [
              ['at_cursor', 'At Cursor'],
              ['at_previous_position', 'At Previous Position'],
              ['center_of_screen', 'Center of Screen'],
            ].map(([v, label]) => ({ label, checked: st.popup_position === v, action: () => saveSettings({ popup_position: v }) })),
          },
          '-',
          { label: 'Keep Window Open', shortcut: 'Ctrl+Space', checked: st.keep_open_on_blur, action: toggleKeepOpen },
          { label: 'Show Thumbnails', checked: st.show_thumbnails, action: () => saveSettings({ show_thumbnails: !st.show_thumbnails }) },
          { label: 'Show Quick Paste Numbers', checked: st.show_quick_paste_numbers, action: () => saveSettings({ show_quick_paste_numbers: !st.show_quick_paste_numbers }) },
        ],
      },
      '-',
      {
        label: 'Connect To Clipboard',
        checked: !S.capturePaused,
        action: async () => {
          S.capturePaused = await call('set_capture_paused', { paused: !S.capturePaused });
          updateHeader();
        },
      },
      { label: 'Save Current Clipboard', action: () => call('save_current_clipboard', {}, 'Saved the current clipboard') },
      { label: 'New Clip', shortcut: `${MOD}N`, action: () => editorDialog(null) },
      { label: 'Import Ditto Database…', action: importDitto },
      '-',
      { label: 'Exit Ditto-RS', action: () => invoke('quit') },
    ];
  }

  function groupMenuItems() {
    const items = [{ label: 'All clips', checked: !inGroup(), action: () => setScope({ kind: 'main' }) }];
    const tree = groupTree();
    if (tree.length) items.push('-');
    for (const { group, depth } of tree) {
      items.push({
        label: `${'    '.repeat(depth)}${group.name}  (${group.clip_count})`,
        checked: inGroup() && S.scope.id === group.id,
        action: () => setScope({ kind: 'group', id: group.id }),
      });
    }
    items.push('-');
    items.push({ label: 'New Group…', shortcut: `${MOD}F7`, action: () => newGroupDialog([]) });
    items.push({ label: 'Rename Group…', disabled: !inGroup(), action: renameGroupDialog });
    items.push({ label: 'Delete Group…', disabled: !inGroup(), action: deleteGroupDialog });
    return items;
  }

  function openContextMenu(x, y) {
    if (!S.items.length) {
      openMenu([{ label: 'New Clip', shortcut: `${MOD}N`, action: () => editorDialog(null) }], x, y);
      return;
    }
    openMenu(clipMenuItems(), x, y);
  }

  async function importDitto() {
    const report = await call('import_ditto_database');
    if (report) {
      toast(`Imported ${report.clips_imported} clip(s), ${report.groups_imported} group(s); skipped ${report.duplicates_skipped} duplicate(s)`);
      await loadGroups();
    }
  }

  // ---- dialogs ----------------------------------------------------------------------------

  let dialogClose = null;

  function openDialog({ title, body, buttons, full = false, initialFocus }) {
    dialogLayer.textContent = '';
    const box = el('div', `dialog${full ? ' full' : ''}`);
    box.setAttribute('role', 'dialog');
    box.setAttribute('aria-modal', 'true');
    if (title) box.append(el('h2', null, title));
    const content = el('div', 'content');
    content.append(body);
    box.append(content);
    const actions = el('div', 'actions');
    const hint = el('span', 'hint', '');
    actions.append(hint);
    let primary = null;
    for (const b of buttons) {
      const btn = el('button', `btn${b.primary ? ' primary' : ''}${b.danger ? ' danger' : ''}`, b.label);
      btn.addEventListener('click', () => b.action());
      actions.append(btn);
      if (b.primary) primary = b;
    }
    box.append(actions);
    dialogLayer.append(box);
    dialogLayer.hidden = false;
    const close = () => {
      dialogLayer.hidden = true;
      dialogLayer.textContent = '';
      dialogClose = null;
      searchEl.focus();
    };
    dialogClose = close;
    box.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        close();
      } else if (e.key === 'Enter' && primary && (e.target.tagName !== 'TEXTAREA' || primaryMod(e))) {
        e.preventDefault();
        e.stopPropagation();
        primary.action();
      } else if (primaryMod(e) && e.key.toLowerCase() === 's' && primary) {
        e.preventDefault();
        primary.action();
      }
      e.stopPropagation();
    });
    const focusTarget = initialFocus || box.querySelector('input, textarea, select, .btn.primary');
    if (focusTarget) setTimeout(() => focusTarget.focus(), 0);
    return { close, hint };
  }

  function confirmDialog({ title, message, confirm, danger, onConfirm }) {
    let dlg;
    dlg = openDialog({
      title,
      body: el('p', 'muted', message),
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        { label: confirm, primary: true, danger, action: async () => { dlg.close(); await onConfirm(); } },
      ],
    });
  }

  async function editClipDialog() {
    const clip = currentClip();
    if (!clip) return;
    if (clip.kind !== 'text') {
      toast('Only text clips can be edited', 'error');
      return;
    }
    const details = await call('clip_details', { id: clip.id });
    editorDialog(clip, details.text || '');
  }

  function editorDialog(clip, text = '') {
    const area = el('textarea');
    area.value = text;
    area.spellcheck = false;
    let dlg;
    const save = async () => {
      const value = area.value;
      try {
        if (clip) await invoke('update_clip_text', { id: clip.id, text: value });
        else await invoke('new_clip', { text: value, group_id: inGroup() ? S.scope.id : null });
        dlg.close();
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    dlg = openDialog({
      title: clip ? 'Edit Clip' : 'New Clip',
      body: area,
      full: true,
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        { label: 'Save', primary: true, action: save },
      ],
      initialFocus: area,
    });
    dlg.hint.textContent = `${isMac ? '⌘' : 'Ctrl+'}Enter saves · Esc cancels`;
  }

  function field(label, input) {
    const wrap = el('label', 'field');
    wrap.append(label, input);
    return wrap;
  }

  async function propertiesDialog() {
    const clip = currentClip();
    if (!clip) return;
    const d = await call('clip_details', { id: clip.id });
    const s = d.summary;
    const body = el('div');
    body.style.display = 'flex';
    body.style.flexDirection = 'column';
    body.style.gap = '8px';

    const title = el('input');
    title.type = 'text';
    title.value = s.description;
    const quick = el('input');
    quick.type = 'text';
    quick.value = s.quick_paste_text || '';
    quick.placeholder = 'Search with /q <text> to find this clip';
    let hotkey = s.hotkey || '';
    const hotkeyBox = D.hotkeyInput(hotkey, (v) => { hotkey = v; });
    const never = el('input');
    never.type = 'checkbox';
    never.checked = s.never_auto_delete;
    const neverLabel = el('label', 'check');
    neverLabel.append(never, 'Never auto delete');
    const group = el('select');
    group.append(new Option('(none)', ''));
    for (const { group: g, depth } of groupTree()) group.append(new Option(`${'  '.repeat(depth)}${g.name}`, String(g.id)));
    group.value = s.group_id != null ? String(s.group_id) : '';

    const meta = el('div', 'meta');
    const metaRows = [
      ['Added', D.formatDate(s.created_at)],
      ['Last used', D.formatDate(s.last_used_at)],
      ['Copied in', s.source_app || '—'],
      ['Formats', d.formats.map((f) => `${f.name} (${D.formatBytes(f.size)})`).join(', ')],
    ];
    for (const [k, v] of metaRows) meta.append(el('span', null, k), el('span', null, v));

    const hotkeyWrap = el('label', 'field');
    hotkeyWrap.append('Global hot key (pastes this clip from any app)', hotkeyBox);
    body.append(field('Item title', title), field('Quick paste text', quick), hotkeyWrap, neverLabel, field('Group', group), meta);

    let dlg;
    const save = async () => {
      try {
        const problems = await invoke('update_clip_properties', {
          id: clip.id,
          properties: { description: title.value, quick_paste_text: quick.value || null, never_auto_delete: never.checked, hotkey: hotkey || null },
        });
        const newGroup = group.value === '' ? null : Number(group.value);
        if (newGroup !== (s.group_id == null ? null : s.group_id)) {
          await invoke('move_to_group', { ids: [clip.id], group_id: newGroup });
        }
        dlg.close();
        problems.forEach((p) => toast(p, 'error'));
      } catch (e) {
        toast(String(e), 'error');
      }
    };
    dlg = openDialog({
      title: 'Clip Properties',
      body,
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        { label: 'Save', primary: true, action: save },
      ],
      initialFocus: title,
    });
  }

  function newGroupDialog(clipIds) {
    const name = el('input');
    name.type = 'text';
    name.placeholder = 'Group name';
    const parent = el('select');
    parent.append(new Option('(top level)', ''));
    for (const { group: g, depth } of groupTree()) parent.append(new Option(`${'  '.repeat(depth)}${g.name}`, String(g.id)));
    if (inGroup()) parent.value = String(S.scope.id);
    const body = el('div');
    body.style.display = 'flex';
    body.style.flexDirection = 'column';
    body.style.gap = '8px';
    body.append(field('Name', name), field('Inside', parent));
    if (clipIds.length) body.append(el('p', 'muted', `${clipIds.length} selected clip(s) will be moved into the new group.`));
    let dlg;
    dlg = openDialog({
      title: clipIds.length ? 'New Group From Selection' : 'New Group',
      body,
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        {
          label: 'Create',
          primary: true,
          action: async () => {
            if (!name.value.trim()) {
              name.focus();
              return;
            }
            try {
              await invoke('create_group', { name: name.value, parent_id: parent.value ? Number(parent.value) : null, clip_ids: clipIds });
              dlg.close();
              toast(`Created group ${name.value.trim()}`);
            } catch (e) {
              toast(String(e), 'error');
            }
          },
        },
      ],
      initialFocus: name,
    });
  }

  function renameGroupDialog() {
    if (!inGroup()) return;
    const name = el('input');
    name.type = 'text';
    name.value = groupName(S.scope.id) || '';
    let dlg;
    dlg = openDialog({
      title: 'Rename Group',
      body: field('Name', name),
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        { label: 'Rename', primary: true, action: async () => { await call('rename_group', { id: S.scope.id, name: name.value }); dlg.close(); } },
      ],
      initialFocus: name,
    });
  }

  function deleteGroupDialog() {
    if (!inGroup()) return;
    const id = S.scope.id;
    const keep = el('input');
    keep.type = 'radio';
    keep.name = 'del';
    keep.checked = true;
    const drop = el('input');
    drop.type = 'radio';
    drop.name = 'del';
    const a = el('label', 'check');
    a.append(keep, 'Keep its clips (they go back to All clips)');
    const b = el('label', 'check');
    b.append(drop, 'Delete its clips too');
    const body = el('div');
    body.append(el('p', 'muted', 'Sub-groups are deleted as well.'), a, b);
    let dlg;
    dlg = openDialog({
      title: `Delete group “${groupName(id)}”?`,
      body,
      buttons: [
        { label: 'Cancel', action: () => dlg.close() },
        {
          label: 'Delete Group',
          primary: true,
          danger: true,
          action: async () => {
            await call('delete_group', { id, delete_clips: drop.checked });
            dlg.close();
            await setScope({ kind: 'main' });
          },
        },
      ],
    });
  }

  // ---- keyboard -----------------------------------------------------------------------------

  function onKeyDown(e) {
    if (!dialogLayer.hidden) return;
    const mod = primaryMod(e);
    const k = e.key;

    // Ditto: Ctrl+1..9, Ctrl+0 paste positions 1–10 (Cmd on macOS also works).
    const digit = /^(Digit|Numpad)([0-9])$/.exec(e.code);
    if (digit && (e.ctrlKey || e.metaKey) && !e.altKey) {
      e.preventDefault();
      const n = Number(digit[2]);
      pastePosition(n === 0 ? 9 : n - 1, e.shiftKey);
      return;
    }

    let handled = true;
    switch (k) {
      case 'ArrowDown': moveCursor(S.cursor + 1, { extend: e.shiftKey }); break;
      case 'ArrowUp': moveCursor(S.cursor - 1, { extend: e.shiftKey }); break;
      case 'PageDown': moveCursor(S.cursor + pageSize(), { extend: e.shiftKey }); break;
      case 'PageUp': moveCursor(S.cursor - pageSize(), { extend: e.shiftKey }); break;
      case 'Home':
        if (mod || !searchEl.value) moveCursor(0, { extend: e.shiftKey });
        else handled = false;
        break;
      case 'End':
        if (mod || !searchEl.value) moveCursor(S.items.length - 1, { extend: e.shiftKey });
        else handled = false;
        break;
      case 'Enter':
        if (e.altKey) propertiesDialog();
        else if (!S.settings.find_as_you_type && searchEl.value !== S.query) applySearch();
        else paste({ plain: e.shiftKey });
        break;
      case 'Escape':
        if (S.preview.open) closePreview();
        else if (searchEl.value) clearSearch();
        else hide();
        break;
      case 'Delete':
        if (e.shiftKey || searchEl.selectionStart === searchEl.value.length) deleteSelection();
        else handled = false;
        break;
      case 'Backspace':
        if (mod && !searchEl.value) deleteSelection();
        else handled = false;
        break;
      case 'F2': editClipDialog(); break;
      case 'F3': togglePreview(); break;
      case 'F5': reload(); break;
      case 'F7':
        if (mod) newGroupDialog([]);
        else newGroupDialog(selectedIds());
        break;
      case 'F10':
        if (e.shiftKey) openContextMenuAtCursor();
        else handled = false;
        break;
      case 'ContextMenu': openContextMenuAtCursor(); break;
      default:
        handled = false;
    }
    if (!handled && e.ctrlKey && (k === ' ' || e.code === 'Space')) {
      toggleKeepOpen();
      handled = true;
    }
    if (!handled && mod && !e.altKey) {
      handled = true;
      switch (k.toLowerCase()) {
        case 'e': editClipDialog(); break;
        case 'n': editorDialog(null); break;
        case 'g': openMenu(groupMenuItems(), 8, 34); break;
        case 'a':
          if (!searchEl.value || document.activeElement !== searchEl) selectAll();
          else handled = false;
          break;
        case 'c':
          if (searchEl.selectionStart === searchEl.selectionEnd || document.activeElement !== searchEl) copySelection(e.shiftKey);
          else handled = false;
          break;
        case ',': invoke('open_settings'); break;
        case 'f': searchEl.focus(); searchEl.select(); break;
        default: handled = false;
      }
    }
    if (handled) {
      e.preventDefault();
      return;
    }
    // Typing anywhere goes to the search box (find as you type).
    if (k.length === 1 && !mod && !e.altKey && document.activeElement !== searchEl) searchEl.focus();
  }

  function openContextMenuAtCursor() {
    const row = rowAt(S.cursor);
    if (row) {
      const r = row.getBoundingClientRect();
      openContextMenu(r.left + 30, Math.min(r.bottom, window.innerHeight - 40));
    } else {
      openContextMenu(30, 60);
    }
  }

  // ---- events -------------------------------------------------------------------------------

  function rowIndexFromEvent(e) {
    const row = e.target.closest('.clip');
    return row ? Number(row.dataset.index) : -1;
  }

  function bindEvents() {
    document.addEventListener('keydown', onKeyDown);

    searchEl.addEventListener('input', () => {
      if (!S.settings.find_as_you_type) return;
      clearTimeout(searchTimer);
      searchTimer = setTimeout(applySearch, 70);
    });

    listEl.addEventListener('mousedown', (e) => {
      // Keep keyboard focus in the search box.
      if (e.button === 0 || e.button === 2) e.preventDefault();
    });
    listEl.addEventListener('click', (e) => {
      const i = rowIndexFromEvent(e);
      if (i < 0) return;
      if (e.shiftKey) moveCursor(i, { extend: true });
      else if (primaryMod(e)) moveCursor(i, { toggle: true });
      else moveCursor(i);
      searchEl.focus();
    });
    listEl.addEventListener('dblclick', (e) => {
      const i = rowIndexFromEvent(e);
      if (i < 0) return;
      moveCursor(i);
      paste({ plain: e.shiftKey });
    });
    listEl.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const i = rowIndexFromEvent(e);
      if (i >= 0 && !S.selected.has(S.items[i].id)) moveCursor(i);
      openContextMenu(e.clientX, e.clientY);
    });
    listEl.addEventListener('scroll', () => {
      if (listEl.scrollTop + listEl.clientHeight > listEl.scrollHeight - 300) loadMore();
    });
    document.addEventListener('contextmenu', (e) => {
      if (!e.target.closest('input, textarea, .preview-body')) e.preventDefault();
    });

    $('group-button').addEventListener('click', (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      openMenu(groupMenuItems(), r.left, r.bottom + 2);
    });
    $('menu-button').addEventListener('click', (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      openMenu(systemMenuItems(), r.left, r.bottom + 2);
    });
    $('pin-button').addEventListener('click', toggleKeepOpen);
    $('mode-button').addEventListener('click', cycleMode);
    $('preview-close').addEventListener('click', closePreview);

    listen('popup-shown', (e) => onShown(e.payload || {}));
    listen('popup-hotkey-again', () => moveCursor(S.cursor + 1));
    listen('clips-changed', scheduleReload);
    listen('groups-changed', async () => {
      await loadGroups();
      updateHeader();
    });
    listen('settings-changed', (e) => {
      S.settings = e.payload;
      D.applyAppearance(S.settings);
      updateHeader();
      updateModeButton();
      renderList();
    });
    listen('capture-changed', (e) => {
      S.capturePaused = !!e.payload;
      updateHeader();
    });
  }

  async function onShown(payload) {
    S.targetApp = payload.target_app || null;
    if (dialogClose) dialogClose();
    if (S.preview.open) closePreview();
    searchEl.value = '';
    S.query = '';
    try {
      S.info = await invoke('app_info');
      S.capturePaused = S.info.capture_paused;
    } catch (e) {
      /* keep the previous info */
    }
    updateHeader();
    updateBanner();
    await reload({ resetCursor: true });
    searchEl.focus();
  }

  async function init() {
    $('group-button').querySelector('.chevron').append(icon('chevron'));
    $('pin-button').append(icon('pin'));
    $('menu-button').append(icon('dots'));
    document.querySelector('.search-icon').append(icon('search'));
    $('preview-close').append(icon('close'));

    S.settings = await invoke('get_settings');
    D.applyAppearance(S.settings);
    [S.transforms, S.info] = await Promise.all([invoke('text_transforms'), invoke('app_info')]);
    S.capturePaused = S.info.capture_paused;
    await loadGroups();
    updateHeader();
    updateModeButton();
    updateBanner();
    bindEvents();
    await reload({ resetCursor: true });
    searchEl.focus();
  }

  init().catch((e) => {
    document.body.textContent = `Ditto-RS could not start its window: ${e}`;
  });
})();
