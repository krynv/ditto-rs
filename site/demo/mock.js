// A stand-in for the Ditto-RS backend, so the app's own quick-paste page can run as the live
// demo on the website. The files in ui/ are used unchanged; this script provides the
// `window.__TAURI__` they talk to and keeps a small sample history in memory. Nothing is saved
// or sent anywhere, and the visitor's real clipboard is never read or written.
(function () {
  'use strict';

  const params = new URLSearchParams(location.search);
  // The page that embeds the demo (site.js). Same origin, so it is called directly.
  const host = (() => {
    try {
      return parent !== window && parent.DittoDemo ? parent.DittoDemo : null;
    } catch (e) {
      return null;
    }
  })();
  const tell = (method, ...args) => (host && typeof host[method] === 'function' ? host[method](...args) : undefined);

  const platform = /Mac|iPhone|iPad/.test(navigator.platform) ? 'macos' : /Win/.test(navigator.platform) ? 'windows' : 'linux';
  const now = Date.now();
  const minutes = (n) => now - n * 60000;

  // ---- sample history -----------------------------------------------------------------------

  // The sample picture is a screenshot of a rail ticket. These are the words on it: the app
  // reads them from the pixels; here they are known.
  const TICKET = {
    title: 'Mobile ticket', fare: 'Adult - Standard',
    from: 'London Kings Cross', to: 'Edinburgh',
    fields: [['Date', 'Sat 5 Dec'], ['Departs', '09:42'], ['Platform', '4'], ['Coach', 'C'], ['Seat', '21']],
    reference: ['Booking reference', 'TK49QX'],
  };
  const PICTURE_TEXT = [
    `${TICKET.title}  ${TICKET.fare}`,
    `${TICKET.from} to ${TICKET.to}`,
    TICKET.fields.map((field) => field.join(' ')).join('  '),
    TICKET.reference.join(' '),
  ].join('\n');

  const picture = (() => {
    const c = document.createElement('canvas');
    c.width = 960;
    c.height = 540;
    const g = c.getContext('2d');
    const ink = '#1d2330';
    const faded = '#6f6656';
    const orange = '#f0841f';
    const type = (weight, size, family) => { g.font = `${weight} ${size}px ${family || 'system-ui, -apple-system, "Segoe UI", sans-serif'}`; };
    g.textBaseline = 'middle';

    g.fillStyle = '#fff8ea';
    g.fillRect(0, 0, 960, 540);
    g.fillStyle = orange;
    g.fillRect(0, 0, 960, 88);
    g.fillRect(0, 516, 960, 24);

    g.fillStyle = '#fff';
    type(600, 34);
    g.fillText(TICKET.title, 56, 46);
    type(500, 28);
    g.textAlign = 'right';
    g.fillText(TICKET.fare, 904, 46);
    g.textAlign = 'left';

    g.fillStyle = ink;
    type(600, 60);
    g.fillText(TICKET.from, 56, 170);
    g.fillText(TICKET.to, 136, 248);
    g.strokeStyle = orange;
    g.lineWidth = 7;
    g.lineCap = 'round';
    g.lineJoin = 'round';
    g.beginPath();
    g.moveTo(60, 248);
    g.lineTo(108, 248);
    g.moveTo(90, 230);
    g.lineTo(108, 248);
    g.lineTo(90, 266);
    g.stroke();

    TICKET.fields.forEach(([label, value], index) => {
      const x = [56, 300, 500, 690, 820][index];
      g.fillStyle = faded;
      type(500, 24);
      g.fillText(label, x, 336);
      g.fillStyle = ink;
      type(600, 40);
      g.fillText(value, x, 382);
    });

    g.strokeStyle = '#d8cbb0';
    g.lineWidth = 3;
    g.setLineDash([12, 10]);
    g.beginPath();
    g.moveTo(56, 432);
    g.lineTo(904, 432);
    g.stroke();
    g.setLineDash([]);

    g.fillStyle = faded;
    type(500, 24);
    g.fillText(TICKET.reference[0], 56, 474);
    g.fillStyle = ink;
    type(600, 30, 'ui-monospace, "SF Mono", "Cascadia Code", Consolas, Menlo, monospace');
    g.fillText(TICKET.reference[1], 56 + 232, 474);
    // A barcode, as printed on the ticket.
    let x = 640;
    for (const width of [4, 2, 6, 2, 2, 4, 8, 2, 4, 2, 6, 4, 2, 2, 8, 2, 4, 6, 2, 4, 2, 2, 6, 4, 2, 8, 2, 4, 2, 6, 2, 4]) {
      g.fillRect(x, 452, width, 44);
      x += width + 4;
    }

    const full = c.toDataURL('image/png');
    const t = document.createElement('canvas');
    t.width = 284;
    t.height = 160;
    t.getContext('2d').drawImage(c, 0, 0, 284, 160);
    return { full, thumbnail: t.toDataURL('image/png'), size: [960, 540] };
  })();

  let nextId = 1;
  function clip(fields) {
    const text = fields.text == null ? null : fields.text;
    const kind = fields.files ? 'files' : fields.image ? 'image' : 'text';
    const formats = [fields.files && 'files', fields.html && 'html', text != null && 'text', fields.image && 'png'].filter(Boolean);
    const description = fields.files ? fields.files.join('\n') : fields.image ? `Image ${picture.size[0]} × ${picture.size[1]}` : text;
    return {
      id: nextId++,
      description,
      kind,
      created_at: minutes(fields.age || 1),
      last_used_at: minutes(fields.age || 1),
      group_id: fields.group_id || null,
      sticky: !!fields.sticky,
      group_sticky: false,
      never_auto_delete: false,
      quick_paste_text: fields.quick_paste_text || null,
      hotkey: null,
      source_app: fields.source_app || null,
      size_bytes: fields.image ? 184320 : new Blob([text || description]).size,
      formats,
      has_thumbnail: !!fields.image,
      // Not part of a real summary: the clip's data.
      text,
      html: fields.html || null,
      files: fields.files || [],
      image: !!fields.image,
    };
  }

  // Top of the list first. Sticky clips stay above the others.
  let clips = [
    clip({ text: 'This is Ditto-RS, running in your browser.\nType to search, then press Enter to paste into the note.', age: 1 }),
    clip({ text: 'https://github.com/krynv/ditto-rs', source_app: 'Browser', age: 3 }),
    clip({ text: 'Best regards,\nAlex Morgan', quick_paste_text: 'sig', source_app: 'Mail', age: 8 }),
    clip({ image: true, source_app: 'Screenshot', age: 14 }),
    clip({ text: 'fn main() {\n    println!("Hello, world!");\n}', source_app: 'Code', age: 21 }),
    clip({ text: 'Meeting moved to Thursday 3 pm, room 4B', html: '<p>Meeting moved to <b>Thursday 3&nbsp;pm</b>, room <i>4B</i></p>', source_app: 'Chat', age: 35 }),
    clip({ files: ['/Users/alex/Documents/Quarterly report.pdf', '/Users/alex/Pictures/diagram.png'], source_app: 'Files', age: 50 }),
    clip({ text: '   Order 4471-2208 arrives Saturday   ', source_app: 'Mail', age: 64 }),
    clip({ text: 'git commit -m "Fix the search box focus"', source_app: 'Terminal', age: 90 }),
    clip({ text: '221B Baker Street, London NW1 6XE', source_app: 'Maps', age: 130 }),
    clip({ text: 'alex.morgan@example.com', source_app: 'Mail', age: 190 }),
    clip({ text: '+44 20 7946 0958', source_app: 'Contacts', age: 240 }),
    clip({ text: 'The quick brown fox jumps over the lazy dog', source_app: 'Notes', age: 400 }),
    clip({ text: '#5a57d6', source_app: 'Design', age: 620 }),
    clip({ text: 'Remember: right-click a clip for Special Paste, groups and more.', group_id: 1, source_app: 'Notes', age: 900 }),
  ];
  let groups = [{ id: 1, name: 'Tips', parent_id: null }];
  let nextGroupId = 2;

  const settings = {
    start_on_login: false, limit_entries: true, max_entries: 500, expire_entries: false, expire_days: 5,
    allow_duplicates: false, description_chars: 500, max_clip_bytes: 0,
    capture: { text: true, html: true, rtf: true, images: true, files: true },
    copy_app_include: '*', copy_app_exclude: '', respect_privacy_markers: true, read_image_text: true,
    hotkey_activate: 'Ctrl+Backquote', hotkey_text_only_paste: '', position_hotkeys: Array(10).fill(''),
    position_hotkeys_send_paste: true, move_to_top_on_paste: true, hotkey_again_moves_selection: true,
    popup_position: 'at_cursor', lines_per_clip: 2, show_quick_paste_numbers: true, show_leading_whitespace: false,
    show_thumbnails: true, find_as_you_type: true, prompt_on_delete: true, show_grouped_in_main: true,
    paste_after_selection: true, multi_paste_separator: '[LF]', multi_paste_reverse: true, keep_open_on_blur: false,
    theme: params.get('theme') || 'system', font_size: 13, ui_zoom: 100, window_width: 420, window_height: 520,
    last_position: null, search_mode: 'wildcard', search_scopes: { description: true, full_text: false, quick_paste: false },
    paste_delay_ms: 120,
  };
  let capturePaused = false;

  // ---- events -------------------------------------------------------------------------------

  const listeners = new Map();
  function emit(event, payload) {
    for (const handler of listeners.get(event) || []) {
      try {
        handler({ event, payload });
      } catch (e) {
        console.error(e);
      }
    }
  }
  const changed = () => emit('clips-changed', null);

  // ---- helpers ------------------------------------------------------------------------------

  const summary = (c) => {
    const { text, html, files, image, ...rest } = c;
    return rest;
  };
  const find = (id) => clips.find((c) => c.id === id);
  const inScope = (c, scope) => (scope && scope.kind === 'group' ? c.group_id === scope.id : settings.show_grouped_in_main || c.group_id == null);
  const plainText = (c) => (c.files.length ? c.files.join('\n') : c.text != null ? c.text : '');

  function toTop(c) {
    clips = clips.filter((x) => x !== c);
    const firstNormal = clips.findIndex((x) => !x.sticky);
    clips.splice(c.sticky ? 0 : firstNormal < 0 ? clips.length : firstNormal, 0, c);
  }

  /** Ditto's search: AND between words, OR / NOT keywords, "phrases", * wildcards; or plain / regex modes. */
  function matcher(query) {
    let scopes = settings.search_scopes;
    const prefix = /^[/\\]([qf]) /.exec(query);
    if (prefix) {
      query = query.slice(3);
      scopes = prefix[1] === 'q' ? { quick_paste: true } : { full_text: true };
    }
    if (!scopes.description && !scopes.full_text && !scopes.quick_paste) scopes = { description: true };
    const trimmed = query.trim();
    if (!trimmed) return null;

    let test;
    if (settings.search_mode === 'contains') {
      const needle = trimmed.toLowerCase();
      test = (s) => s.toLowerCase().includes(needle);
    } else if (settings.search_mode === 'regex') {
      const re = new RegExp(trimmed, 'i');
      test = (s) => re.test(s);
    } else {
      const groups = [[]];
      let negate = false;
      let or = false;
      for (const m of query.replace(/[[\]]/g, ' ').matchAll(/"([^"]*)"|(\S+)/g)) {
        const quoted = m[1] !== undefined;
        const word = quoted ? m[1] : m[2];
        if (!word.trim()) continue;
        const keyword = quoted ? '' : word.toUpperCase();
        if (keyword === 'NOT' || keyword === '!') { negate = true; continue; }
        if (keyword === 'OR') { or = true; continue; }
        if (keyword === 'AND') { or = false; continue; }
        const pattern = word.includes('*')
          ? new RegExp(word.split('*').map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.*'), 'i')
          : word.toLowerCase();
        if (or && groups[groups.length - 1].length) groups.push([]);
        groups[groups.length - 1].push({ negate, pattern });
        negate = false;
        or = false;
      }
      if (!groups.some((g) => g.length)) return null;
      test = (s) => {
        const lower = s.toLowerCase();
        return groups.some((g) => g.length && g.every((t) => (typeof t.pattern === 'string' ? lower.includes(t.pattern) : t.pattern.test(s)) !== t.negate));
      };
    }
    return (c) => (scopes.description && test(c.description))
      || (scopes.quick_paste && c.quick_paste_text != null && test(c.quick_paste_text))
      || (scopes.full_text && c.text != null && test(c.text))
      // As in search.rs: the text read from a picture counts under both of these scopes.
      || ((scopes.description || scopes.full_text) && c.image && test(PICTURE_TEXT));
  }

  // "Special Paste", as in transform.rs.
  const words = (s, f) => s.replace(/\S+/g, f);
  const TRANSFORMS = [
    ['upper_case', 'UPPER CASE', (s) => s.toUpperCase()],
    ['lower_case', 'lower case', (s) => s.toLowerCase()],
    ['capitalize', 'Capitalise Case', (s) => words(s.toLowerCase(), (w) => w[0].toUpperCase() + w.slice(1))],
    ['sentence_case', 'Sentence case', (s) => s.toLowerCase().replace(/(^\s*|[.!?]\s+)(\p{L})/gu, (m, a, b) => a + b.toUpperCase())],
    ['invert_case', 'Invert Case', (s) => [...s].map((ch) => (ch === ch.toUpperCase() ? ch.toLowerCase() : ch.toUpperCase())).join('')],
    ['trim_white_space', 'Trim White Space', (s) => s.trim()],
    ['camel_case', 'CamelCase', (s) => s.split(/\s+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join('')],
    ['ascii_only', 'Ascii Text Only', (s) => s.replace(/[^\x00-\x7f]/g, '')],
    ['remove_line_feeds', 'Remove Line Feeds', (s) => s.replace(/\r?\n/g, ' ')],
    ['add_one_line_feed', 'Paste + One Line Feed', (s) => `${s}\n`],
    ['add_two_line_feeds', 'Paste + Two Line Feeds', (s) => `${s}\n\n`],
    ['add_current_time', 'Paste + Current Time', (s) => `${s}\n${new Date().toLocaleString()}`],
    ['typoglycemia', 'Typoglycemia', (s) => words(s, (w) => {
      if (w.length <= 3) return w;
      const inner = [...w.slice(1, -1)].sort(() => Math.random() - 0.5).join('');
      return w[0] + inner + w[w.length - 1];
    })],
    ['slugify', 'Slugify', (s) => s.toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '')],
    ['posixify_paths', 'Posixify Paths', (s) => s.replace(/\\/g, '/')],
  ];

  const separator = () => settings.multi_paste_separator.replace(/\[LF\]/g, '\n').replace(/\[CR\]/g, '\r').replace(/\[TAB\]/g, '\t');

  // ---- commands (the names and shapes of app/src/commands.rs) ---------------------------------

  const commands = {
    get_settings: () => ({ ...settings }),
    save_settings: ({ settings: next }) => {
      Object.assign(settings, next);
      emit('settings-changed', { ...settings });
      return { settings: { ...settings }, warnings: [] };
    },
    app_info: () => ({
      version: 'demo', os: platform, data_dir: '(this browser tab)', db_path: '(this browser tab)', portable: false,
      can_paste: true, zoom_supported: false, capture_paused: capturePaused, hotkey_problems: [],
    }),
    text_transforms: () => TRANSFORMS.map(([id, label]) => ({ id, label })),

    list_clips: ({ request }) => {
      let match;
      try {
        match = matcher(request.query || '');
      } catch (e) {
        return { items: [], has_more: false, search_error: String(e.message || e) };
      }
      const found = clips.filter((c) => inScope(c, request.scope) && (!match || match(c)));
      const page = found.slice(request.offset, request.offset + request.limit);
      return { items: page.map(summary), has_more: request.offset + page.length < found.length, search_error: null };
    },
    clip_thumbnail: ({ id }) => (find(id) && find(id).image ? picture.thumbnail : null),
    clip_details: ({ id }) => {
      const c = find(id);
      if (!c) throw new Error(`clip ${id} not found`);
      const sizes = { text: new Blob([c.text || '']).size, html: new Blob([c.html || '']).size, files: new Blob([c.files.join('\n')]).size, png: 184320 };
      return {
        summary: summary(c),
        text: c.text,
        text_truncated: false,
        html: c.html,
        rtf_text: null,
        image: c.image ? picture.full : null,
        image_size: c.image ? picture.size : null,
        image_text: c.image ? PICTURE_TEXT : null,
        files: c.files,
        formats: c.formats.map((name) => ({ name, size: sizes[name] })),
      };
    },

    paste_clips: ({ request }) => {
      let chosen = request.ids.map(find).filter(Boolean);
      if (!chosen.length) return null;
      // Several clips are pasted oldest first, with the separator between them.
      if (chosen.length > 1 && settings.multi_paste_reverse) chosen = chosen.slice().sort((a, b) => clips.indexOf(b) - clips.indexOf(a));
      const picture_only = chosen.every((c) => c.image);
      let text = chosen.filter((c) => !c.image).map(plainText).join(separator());
      const transform = TRANSFORMS.find(([id]) => id === request.transform);
      if (transform) text = transform[2](text);
      if (!request.keep_order && settings.move_to_top_on_paste) chosen.slice().reverse().forEach(toTop);
      chosen.forEach((c) => { c.last_used_at = Date.now(); });
      changed();
      tell('pasted', { text, picture: picture_only, count: chosen.length, keepOpen: settings.keep_open_on_blur });
      return null;
    },
    copy_clips: ({ ids }) => {
      tell('note', ids.length > 1 ? `${ids.length} clips are on the clipboard` : 'The clip is on the clipboard');
      return null;
    },
    delete_clips: ({ ids }) => {
      const before = clips.length;
      clips = clips.filter((c) => !ids.includes(c.id));
      changed();
      return before - clips.length;
    },
    new_clip: ({ text, group_id }) => {
      const c = clip({ text, group_id, source_app: 'Ditto-RS' });
      c.created_at = c.last_used_at = Date.now();
      toTop(c);
      changed();
      return c.id;
    },
    update_clip_text: ({ id, text }) => {
      const c = find(id);
      if (c) Object.assign(c, { text, description: text, html: null, formats: ['text'] });
      changed();
      return null;
    },
    update_clip_properties: ({ id, properties }) => {
      const c = find(id);
      if (c) {
        Object.assign(c, {
          description: properties.description || c.description,
          quick_paste_text: properties.quick_paste_text || null,
          never_auto_delete: !!properties.never_auto_delete,
          hotkey: properties.hotkey || null,
        });
      }
      changed();
      return [];
    },
    set_sticky: ({ id, op }) => {
      const c = find(id);
      if (!c) return null;
      if (op === 'replace_top') clips.forEach((x) => { x.sticky = false; });
      c.sticky = op !== 'remove';
      clips = clips.filter((x) => x !== c);
      const stickies = clips.filter((x) => x.sticky).length;
      clips.splice(!c.sticky ? stickies : op === 'last' ? stickies : 0, 0, c);
      changed();
      return null;
    },
    move_clip: ({ id, op }) => {
      const c = find(id);
      if (!c) return null;
      const peers = clips.filter((x) => x.sticky === c.sticky);
      const i = peers.indexOf(c);
      const target = { top: 0, up: Math.max(0, i - 1), down: Math.min(peers.length - 1, i + 1), last: peers.length - 1 }[op];
      peers.splice(i, 1);
      peers.splice(target, 0, c);
      clips = [...(c.sticky ? peers : clips.filter((x) => x.sticky)), ...(c.sticky ? clips.filter((x) => !x.sticky) : peers)];
      changed();
      return null;
    },

    list_groups: () => groups.map((g) => ({ ...g, clip_count: clips.filter((c) => c.group_id === g.id).length })),
    create_group: ({ name, parent_id, clip_ids }) => {
      const g = { id: nextGroupId++, name, parent_id: parent_id || null };
      groups.push(g);
      clips.forEach((c) => { if ((clip_ids || []).includes(c.id)) c.group_id = g.id; });
      emit('groups-changed', null);
      changed();
      return g.id;
    },
    rename_group: ({ id, name }) => {
      const g = groups.find((x) => x.id === id);
      if (g) g.name = name;
      emit('groups-changed', null);
      return null;
    },
    delete_group: ({ id, delete_clips }) => {
      groups = groups.filter((g) => g.id !== id);
      if (delete_clips) clips = clips.filter((c) => c.group_id !== id);
      else clips.forEach((c) => { if (c.group_id === id) c.group_id = null; });
      emit('groups-changed', null);
      changed();
      return null;
    },
    move_to_group: ({ ids, group_id }) => {
      clips.forEach((c) => { if (ids.includes(c.id)) c.group_id = group_id; });
      emit('groups-changed', null);
      changed();
      return null;
    },

    set_capture_paused: ({ paused }) => {
      capturePaused = paused;
      emit('capture-changed', paused);
      return paused;
    },
    toggle_keep_open: () => {
      settings.keep_open_on_blur = !settings.keep_open_on_blur;
      emit('settings-changed', { ...settings });
      return settings.keep_open_on_blur;
    },
    hide_popup: () => { tell('hide'); return null; },
    quit: () => { tell('hide'); return null; },
    context_menu_opening: () => null,
    request_paste_permission: () => true,

    // Things that need the real app.
    open_settings: () => { tell('note', 'In the app, this opens the Options window.'); return null; },
    import_ditto_database: () => { tell('note', 'In the app, this imports your clips from Ditto on Windows.'); return null; },
    save_clip_to_file: () => { tell('note', 'In the app, this saves the clip as a file.'); return null; },
    save_current_clipboard: () => { tell('note', 'In the app, this saves what is on the clipboard right now.'); return null; },
  };

  async function invoke(command, args) {
    const handler = commands[command];
    if (!handler) {
      console.warn('[demo] not available here:', command);
      return null;
    }
    return handler(args || {});
  }

  // ---- menus --------------------------------------------------------------------------------
  // The app uses native menus. Here the embedding page draws them, so that, like native menus,
  // they can extend past the edge of the quick-paste window.

  const Menu = {
    async new({ items }) {
      return {
        async popup(position) {
          tell('menu', items, position ? position.x : 0, position ? position.y : 0);
        },
        async close() {},
      };
    },
  };

  window.__TAURI__ = {
    core: { invoke },
    event: {
      async listen(event, handler) {
        if (!listeners.has(event)) listeners.set(event, []);
        listeners.get(event).push(handler);
        return () => listeners.set(event, listeners.get(event).filter((h) => h !== handler));
      },
    },
    menu: { Menu },
    dpi: { LogicalPosition: class { constructor(x, y) { this.x = x; this.y = y; } } },
  };

  // ---- what the embedding page can ask for --------------------------------------------------

  window.DittoDemoBackend = {
    /** The window was opened again (the hot key): the app resets its search and selection. */
    shown() {
      emit('popup-shown', { has_target: true, target_app: 'Notes' });
    },
    /** Something was copied in the sample note: it joins the history, like any copy. */
    copied(text) {
      if (capturePaused || !text.trim()) return false;
      const existing = clips.find((c) => c.text === text && !c.image && !c.files.length);
      const c = existing || clip({ text, source_app: 'Notes' });
      c.created_at = c.last_used_at = Date.now();
      toTop(c);
      changed();
      return true;
    },
    /** "Keep the window open" is switched on (the pin button). */
    keepOpen() {
      return settings.keep_open_on_blur;
    },
  };

  // In the app the hot key is global. While this window has the focus, pressing it again
  // moves the selection down (Ditto does the same); Escape and losing focus hide the window.
  window.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.code === 'Backquote') {
      e.preventDefault();
      emit('popup-hotkey-again', null);
    }
  }, true);
})();
