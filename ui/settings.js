// Options window: one page per Ditto options tab, edited as a draft and saved together.
'use strict';

(function () {
  const D = window.Ditto;
  const { invoke, listen, el, toast } = D;

  let saved = null;
  let draft = null;
  let info = null;
  let current = 'general';
  const pageEl = document.getElementById('page');

  const clone = (v) => JSON.parse(JSON.stringify(v));
  const get = (obj, path) => path.split('.').reduce((o, k) => (o == null ? o : o[k]), obj);
  const set = (obj, path, value) => {
    const keys = path.split('.');
    let o = obj;
    keys.slice(0, -1).forEach((k) => { o = o[k]; });
    o[keys[keys.length - 1]] = value;
  };

  // ---- form controls --------------------------------------------------------------------

  function changed(path, value) {
    set(draft, path, value);
    if (path === 'theme' || path === 'font_size') D.applyAppearance(draft);
    refreshDependents();
    updateSaveBar();
  }

  // [element, path]: the element is enabled only while the setting at `path` is on.
  const dependents = [];
  function refreshDependents() {
    for (const [node, path] of dependents) node.disabled = !get(draft, path);
  }

  function check(path, label, hint) {
    const row = el('div', 'row');
    const lab = el('label', 'check');
    const input = el('input');
    input.type = 'checkbox';
    input.checked = !!get(draft, path);
    input.addEventListener('change', () => changed(path, input.checked));
    lab.append(input, label);
    row.append(lab);
    if (hint) row.append(el('p', 'hint', hint));
    return row;
  }

  function numberInput(path, { min = 0, max = 1e9, step = 1, scale = 1 } = {}) {
    const input = el('input');
    input.type = 'number';
    input.min = String(min);
    input.max = String(max);
    input.step = String(step);
    input.value = String(Math.round((get(draft, path) / scale) * 100) / 100);
    input.addEventListener('change', () => {
      let v = Number(input.value);
      if (!Number.isFinite(v)) v = min;
      v = Math.min(max, Math.max(min, v));
      input.value = String(v);
      changed(path, Math.round(v * scale));
    });
    return input;
  }

  function number(path, label, opts = {}) {
    const row = el('div', 'row');
    row.append(el('span', 'label', label), numberInput(path, opts));
    if (opts.suffix) row.append(el('span', 'suffix', opts.suffix));
    return row;
  }

  /** A checkbox followed by the number it enables ("Maximum number of saved copies [500]"). */
  function checkNumber(flagPath, label, numPath, opts = {}) {
    const row = el('div', 'row');
    const lab = el('label', 'check label');
    const box = el('input');
    box.type = 'checkbox';
    box.checked = !!get(draft, flagPath);
    box.addEventListener('change', () => changed(flagPath, box.checked));
    lab.append(box, label);
    const input = numberInput(numPath, opts);
    dependents.push([input, flagPath]);
    row.append(lab, input);
    if (opts.suffix) row.append(el('span', 'suffix', opts.suffix));
    return row;
  }

  /** Destructive actions need a second click within a few seconds. */
  function confirmClick(button, fn) {
    const label = button.textContent;
    let armed = false;
    let timer = null;
    button.addEventListener('click', async () => {
      if (!armed) {
        armed = true;
        button.textContent = 'Click again to confirm';
        timer = setTimeout(() => { armed = false; button.textContent = label; }, 3500);
        return;
      }
      clearTimeout(timer);
      armed = false;
      button.textContent = label;
      await fn();
    });
  }

  function text(path, label, { placeholder = '', hint } = {}) {
    const row = el('div', 'row');
    row.append(el('span', 'label', label));
    const input = el('input');
    input.type = 'text';
    input.placeholder = placeholder;
    input.value = get(draft, path) || '';
    input.spellcheck = false;
    input.addEventListener('input', () => changed(path, input.value));
    row.append(input);
    if (hint) row.append(el('p', 'hint', hint));
    return row;
  }

  function select(path, label, options) {
    const row = el('div', 'row');
    row.append(el('span', 'label', label));
    const input = el('select');
    for (const [value, text] of options) input.append(new Option(text, String(value)));
    input.value = String(get(draft, path));
    input.addEventListener('change', () => {
      const raw = input.value;
      const typed = options.find(([v]) => String(v) === raw);
      changed(path, typed ? typed[0] : raw);
    });
    row.append(input);
    return row;
  }

  function hotkey(path, label) {
    const row = el('div', 'row');
    row.append(el('span', 'label', label));
    row.append(D.hotkeyInput(get(draft, path), (v) => changed(path, v)));
    return row;
  }

  function section(title, ...children) {
    const box = el('section', 'section');
    if (title) box.append(el('h2', null, title));
    box.append(...children.filter(Boolean));
    return box;
  }

  function header(title, lead) {
    const frag = document.createDocumentFragment();
    frag.append(el('h1', null, title));
    if (lead) frag.append(el('p', 'lead', lead));
    return frag;
  }

  /** "Interface size" zooms both windows. Unlike the other options it applies at once. */
  async function setZoom(percent) {
    if (percent === saved.ui_zoom) return;
    try {
      adoptAppliedSettings(await invoke('set_ui_zoom', { percent }));
      render();
      updateSaveBar();
    } catch (e) {
      updateSaveBar(String(e), 'error');
    }
  }

  /** Settings the app changed by itself are not part of the unsaved edits. */
  function adoptAppliedSettings(applied) {
    saved = applied;
    for (const key of ['ui_zoom', 'window_width', 'window_height', 'last_position']) draft[key] = applied[key];
  }

  function interfaceSize() {
    if (!info.zoom_supported) return null;
    const row = el('div', 'row');
    row.append(el('span', 'label', 'Interface size'));
    const input = el('select');
    const sizes = [...new Set([...D.ZOOM_STEPS, saved.ui_zoom])].sort((a, b) => a - b);
    for (const z of sizes) input.append(new Option(`${z}%`, String(z)));
    input.value = String(saved.ui_zoom);
    input.addEventListener('change', () => setZoom(Number(input.value)));
    const mod = D.isMac ? '⌘' : 'Ctrl+';
    row.append(input, el('p', 'hint', `Makes everything larger or smaller: text, pictures and both windows. It applies straight away. You can also press ${mod}+ and ${mod}− in either window.`));
    return row;
  }

  /** Which engine reads the text in images here, and how far it has got. */
  function imageTextStatus() {
    const line = el('p', 'hint', '');
    invoke('image_text_status')
      .then((s) => {
        if (!s.available) {
          line.textContent = s.detail;
          return;
        }
        const progress = s.waiting
          ? ` Reading ${s.waiting.toLocaleString()} more ${s.waiting === 1 ? 'image' : 'images'} in the background.`
          : s.read ? ` ${s.read.toLocaleString()} ${s.read === 1 ? 'image' : 'images'} read.` : '';
        line.textContent = `${s.engine}: ${s.detail}${progress}`;
      })
      .catch(() => {});
    return line;
  }

  // ---- pages ----------------------------------------------------------------------------

  function pageGeneral() {
    const types = el('div', 'chips');
    types.append(
      check('capture.text', 'Plain text'),
      check('capture.html', 'HTML'),
      check('capture.rtf', 'Rich text (RTF)'),
      check('capture.images', 'Images'),
      check('capture.files', 'File lists'),
    );
    const imageText = check('read_image_text', 'Read the text in images so search can find them', 'Screenshots and other image clips are read on this computer. Nothing is sent anywhere.');
    imageText.append(imageTextStatus());
    return [
      header('General', 'What Ditto-RS saves, and for how long.'),
      section(null, check('start_on_login', 'Start Ditto-RS when you log in')),
      section(
        'History',
        checkNumber('limit_entries', 'Maximum number of saved copies', 'max_entries', { min: 1, max: 1000000 }),
        checkNumber('expire_entries', 'Delete copies not pasted for', 'expire_days', { min: 1, max: 36500, suffix: 'days' }),
        el('p', 'hint', 'Clips that are sticky, in a group, have a hot key or are marked "never auto delete" are always kept.'),
        check('allow_duplicates', 'Keep duplicate copies', 'Off: copying something that is already saved moves the existing clip to the top.'),
        number('max_clip_bytes', 'Skip copies larger than', { min: 0, max: 10000, step: 1, scale: 1048576, suffix: 'MB (0 = no limit)' }),
        number('description_chars', 'Characters shown in the list', { min: 16, max: 100000 }),
      ),
      section('Supported types', types),
      section('Text in images', imageText),
      section(
        'Applications',
        text('copy_app_include', 'Only save copies made in', { placeholder: '*' }),
        text('copy_app_exclude', 'Never save copies made in', {
          placeholder: 'e.g. KeePassXC; 1Password*',
          hint: 'Separate names with ; — * and ? are wildcards. Names look like chrome.exe (Windows), Safari (macOS) or firefox (Linux).',
        }),
        check('respect_privacy_markers', 'Skip copies that password managers mark as secret', 'Honours Clipboard Viewer Ignore / ExcludeClipboardContentFromMonitorProcessing (Windows), org.nspasteboard.ConcealedType (macOS) and x-kde-passwordManagerHint (Linux).'),
      ),
    ];
  }

  function pageKeyboard() {
    const grid = el('div', 'grid2');
    for (let i = 0; i < 10; i++) grid.append(hotkey(`position_hotkeys.${i}`, `Position ${i + 1}`));
    const keys = el('table', 'keys');
    const mod = D.isMac ? '⌘' : 'Ctrl+';
    const rows = [
      ['Enter / double-click', 'Paste the selected clip(s)'],
      ['Shift+Enter', 'Paste as plain text'],
      [`${mod}1 … ${mod}0`, 'Paste clip 1–10'],
      ['Shift+↑ / Shift+↓, Shift/Ctrl+click', 'Select several clips (pasted together)'],
      ['Delete', 'Delete the selected clip(s)'],
      [`F2 or ${mod}E`, 'Edit the clip'],
      ['F3', 'View the full clip'],
      [`${mod}N`, 'New clip'],
      [`F7 / ${mod}F7`, 'New group from the selection / new group'],
      [`${mod}G`, 'Switch group'],
      [`${mod}C`, 'Copy to the clipboard without pasting'],
      [D.isMac ? '⌥Enter' : 'Alt+Enter', 'Clip properties'],
      ['Ctrl+Space', 'Keep the window open'],
      ['Esc', 'Clear the search, then close'],
    ];
    for (const [k, v] of rows) {
      const tr = el('tr');
      const kd = el('td');
      kd.append(el('kbd', null, k));
      tr.append(kd, el('td', null, v));
      keys.append(tr);
    }
    return [
      header('Keyboard', 'Global hot keys work in every app. Click a field and press the combination; Backspace clears it.'),
      section(
        'Global hot keys',
        hotkey('hotkey_activate', 'Open Ditto-RS'),
        hotkey('hotkey_text_only_paste', 'Paste clipboard as plain text'),
        check('hotkey_again_moves_selection', 'Pressing "Open Ditto-RS" again moves the selection down'),
        check('move_to_top_on_paste', 'Move clips to the top of the list when pasted'),
      ),
      section(
        'Paste positions (global hot keys for the last ten copies)',
        grid,
        check('position_hotkeys_send_paste', 'Paste right away (otherwise only put the clip on the clipboard)'),
      ),
      section('In the quick-paste window', keys),
      info && info.os === 'linux'
        ? section(null, el('p', 'hint', 'Wayland does not let apps register global hot keys. Bind “ditto-rs --show” to a shortcut in your desktop keyboard settings instead.'))
        : null,
    ];
  }

  function pageQuickPaste() {
    const scopes = el('div', 'chips');
    scopes.append(
      check('search_scopes.description', 'Description'),
      check('search_scopes.full_text', 'Full text'),
      check('search_scopes.quick_paste', 'Quick paste text'),
    );
    return [
      header('Quick Paste', 'The window that opens with the hot key.'),
      section(
        'Appearance',
        select('theme', 'Theme', [['system', 'Same as the system'], ['light', 'Light'], ['dark', 'Dark']]),
        interfaceSize(),
        number('font_size', 'Clip text size', { min: 9, max: 24, suffix: 'px' }),
        number('lines_per_clip', 'Text lines per clip', { min: 1, max: 5 }),
        select('popup_position', 'Open the window', [['at_cursor', 'At the mouse cursor'], ['at_previous_position', 'Where it was last'], ['center_of_screen', 'In the middle of the screen']]),
        check('show_quick_paste_numbers', 'Show numbers for the first ten clips'),
        check('show_thumbnails', 'Show thumbnails for images'),
        check('show_leading_whitespace', 'Show leading whitespace'),
      ),
      section(
        'Behaviour',
        check('paste_after_selection', 'Paste into the previous app after choosing a clip', 'Off: the clip is only put on the clipboard.'),
        number('paste_delay_ms', 'Wait before pasting', { min: 0, max: 2000, step: 10, suffix: 'ms' }),
        check('find_as_you_type', 'Find as you type'),
        check('prompt_on_delete', 'Ask before deleting clips'),
        check('show_grouped_in_main', 'Show clips that are in groups in the main list'),
        check('keep_open_on_blur', 'Keep the window open when it loses focus'),
      ),
      section(
        'Pasting several clips',
        text('multi_paste_separator', 'Separator', { hint: '[LF] = new line, [TAB] = tab, [CR] = carriage return.' }),
        check('multi_paste_reverse', 'Paste the oldest clip first'),
      ),
      section(
        'Search',
        select('search_mode', 'Mode', [['wildcard', 'Wildcard (AND / OR / NOT, "phrases", *)'], ['contains', 'Contains text'], ['regex', 'Regular expression']]),
        el('p', 'hint', 'Search in:'),
        scopes,
        el('p', 'hint', 'Start a search with /f to search the full text, or /q for quick paste text.'),
      ),
    ];
  }

  function stat(name, value) {
    const box = el('div', 'stat');
    box.append(el('div', 'value', value), el('div', 'name', name));
    return box;
  }

  async function pageDatabase() {
    const stats = await invoke('stats');
    const grid = el('div', 'stats');
    grid.append(
      stat('Clips', String(stats.clips)),
      stat('Groups', String(stats.groups)),
      stat('Clip data', D.formatBytes(stats.data_bytes)),
      stat('Database file', D.formatBytes(stats.db_bytes)),
      stat('Copies saved', String(stats.total_copies)),
      stat('Pastes', String(stats.total_pastes)),
    );
    const since = el('p', 'hint', `Counting since ${D.formatDate(stats.since)}${stats.oldest_clip ? ` · oldest clip from ${D.formatDate(stats.oldest_clip)}` : ''}.`);

    const where = el('div');
    where.append(el('p', 'hint', 'History'), el('div', 'path', info.db_path));

    const buttons = el('div', 'buttons');
    const run = (b, fn) => async () => {
      b.disabled = true;
      try {
        await fn();
      } catch (e) {
        toast(String(e), 'error');
      } finally {
        b.disabled = false;
      }
    };
    const button = (label, fn, { cls, confirm } = {}) => {
      const b = el('button', `btn${cls ? ` ${cls}` : ''}`, label);
      if (confirm) confirmClick(b, run(b, fn));
      else b.addEventListener('click', run(b, fn));
      buttons.append(b);
    };
    button('Compact Database', async () => {
      await invoke('compact_database');
      toast('Database compacted');
      render();
    });
    button('Backup Database', async () => {
      const path = await invoke('backup_database');
      toast(`Backup written to ${path}`);
    });
    button('Import Ditto Database…', async () => {
      const r = await invoke('import_ditto_database');
      if (r) {
        toast(`Imported ${r.clips_imported} clip(s) and ${r.groups_imported} group(s); skipped ${r.duplicates_skipped} duplicate(s)`);
        render();
      }
    });
    button('Clear History', async () => {
      const n = await invoke('clear_history');
      toast(`Deleted ${n} clip(s); sticky, grouped, hot-keyed and protected clips were kept`);
      render();
    }, { cls: 'danger', confirm: true });

    const portable = info.portable ? el('span', 'badge', 'Portable mode') : null;
    return [
      header('Database', 'Your clips are stored locally in a SQLite database.'),
      section('Statistics', grid, since),
      section('Location', where, portable, el('p', 'hint', `Data folder: ${info.data_dir}`), buttons),
      section(null, el('p', 'hint', 'Moving from Ditto on Windows? “Import Ditto Database…” reads Ditto.db (usually in %APPDATA%\\Ditto) and brings over clips, groups, sticky clips, quick paste text and hot keys.')),
    ];
  }

  function pageAbout() {
    const perm = el('div', 'row');
    if (info.os === 'macos') {
      perm.append(el('span', 'label', 'Paste into other apps'));
      perm.append(el('span', info.can_paste ? 'ok' : 'bad', info.can_paste ? 'Allowed' : 'Accessibility access needed'));
      if (!info.can_paste) {
        const b = el('button', 'btn', 'Allow…');
        b.addEventListener('click', async () => {
          await invoke('request_paste_permission');
          info = await invoke('app_info');
          render();
        });
        perm.append(b);
      }
    }
    const problems = info.hotkey_problems.length ? el('ul', 'warning-list') : null;
    if (problems) info.hotkey_problems.forEach((p) => problems.append(el('li', null, p)));
    const credit = el('p');
    credit.append('Ditto-RS is an independent, cross-platform re-creation of ');
    credit.append(el('strong', null, 'Ditto'));
    credit.append(' (github.com/sabrogden/Ditto), the Windows clipboard manager by Scott Brogden and contributors, licensed under the GPL-3.0. It is written in Rust with Tauri.');
    return [
      header('About', null),
      section(
        null,
        el('p', null, `Ditto-RS ${info.version} · ${info.os}`),
        credit,
        perm.childNodes.length ? perm : null,
        problems ? el('p', 'hint', 'Hot keys that could not be registered:') : null,
        problems,
      ),
    ];
  }

  const PAGES = [
    ['general', 'General', pageGeneral],
    ['keyboard', 'Keyboard', pageKeyboard],
    ['quick', 'Quick Paste', pageQuickPaste],
    ['database', 'Database', pageDatabase],
    ['about', 'About', pageAbout],
  ];

  let shownPage = null;

  async function render() {
    dependents.length = 0;
    const page = PAGES.find(([id]) => id === current);
    const nodes = await page[2]();
    // Redrawing the page that is already shown must not jump back to its top.
    const scrollTop = shownPage === current ? pageEl.scrollTop : 0;
    pageEl.textContent = '';
    pageEl.append(...nodes.filter(Boolean));
    pageEl.scrollTop = scrollTop;
    shownPage = current;
    refreshDependents();
    for (const b of document.querySelectorAll('#nav button')) {
      b.setAttribute('aria-current', b.dataset.page === current ? 'page' : 'false');
    }
  }

  function renderNav() {
    const nav = document.getElementById('nav');
    const brand = el('div', 'brand');
    const logo = el('img');
    logo.src = 'icon.png';
    logo.alt = '';
    brand.append(logo, 'Ditto-RS');
    nav.append(brand);
    for (const [id, title] of PAGES) {
      const b = el('button', null, title);
      b.dataset.page = id;
      b.addEventListener('click', () => {
        current = id;
        render();
      });
      nav.append(b);
    }
  }

  // ---- save bar --------------------------------------------------------------------------

  const isDirty = () => JSON.stringify(draft) !== JSON.stringify(saved);

  function updateSaveBar(message, kind) {
    const dirty = isDirty();
    document.getElementById('save').disabled = !dirty;
    document.getElementById('revert').disabled = !dirty;
    const status = document.getElementById('save-status');
    status.className = 'save-status';
    if (message) {
      status.textContent = message;
      if (kind) status.classList.add(kind);
    } else if (dirty) {
      status.textContent = 'Unsaved changes';
      status.classList.add('dirty');
    } else {
      status.textContent = '';
    }
  }

  async function save() {
    try {
      const res = await invoke('save_settings', { settings: draft });
      saved = res.settings;
      draft = clone(saved);
      info = await invoke('app_info');
      D.applyAppearance(saved);
      if (res.warnings.length) updateSaveBar(`Saved, but: ${res.warnings.join(' · ')}`, 'error');
      else updateSaveBar('Saved');
      render();
    } catch (e) {
      updateSaveBar(String(e), 'error');
    }
  }

  async function init() {
    saved = await invoke('get_settings');
    draft = clone(saved);
    info = await invoke('app_info');
    D.applyAppearance(saved);
    renderNav();
    await render();
    updateSaveBar();
    document.getElementById('save').addEventListener('click', save);
    document.getElementById('revert').addEventListener('click', () => {
      draft = clone(saved);
      D.applyAppearance(saved);
      render();
      updateSaveBar();
    });
    confirmClick(document.getElementById('defaults'), async () => {
      try {
        const res = await invoke('reset_settings');
        saved = res.settings;
        draft = clone(saved);
        D.applyAppearance(saved);
        render();
        updateSaveBar('Defaults restored');
      } catch (e) {
        updateSaveBar(String(e), 'error');
      }
    });
    document.addEventListener('keydown', (e) => {
      const zoomStep = info.zoom_supported ? D.zoomKey(e) : 0;
      if (zoomStep) {
        e.preventDefault();
        setZoom(D.nextZoom(saved.ui_zoom, zoomStep));
      } else if ((D.isMac ? e.metaKey : e.ctrlKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        if (isDirty()) save();
      }
    });
    listen('settings-changed', (e) => {
      // Changed from the quick-paste window: adopt it unless we have local edits.
      if (!isDirty()) {
        saved = e.payload;
        draft = clone(saved);
      } else {
        adoptAppliedSettings(e.payload);
      }
      render();
      updateSaveBar();
    });
  }

  init().catch((e) => {
    pageEl.textContent = `Could not load the options: ${e}`;
  });
})();
