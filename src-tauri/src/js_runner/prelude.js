// Playground prelude: evaluated (as "badger_prelude") before every run, then called with the
// host hooks. Installs `console` and `note`, and returns the formatter the runner uses for
// the last value and for errors.
//
// User code runs after this and may patch anything (Array.prototype.map = null, getters on
// Object.prototype, Proxies). So every intrinsic is captured here and called through the
// captured Reflect.apply; property reads go through own-property descriptors (getters are
// never invoked); results are built by string concatenation (no array writes that could hit
// a setter on Array.prototype). Every formatting step is wrapped so a hostile value falls
// back to a type tag instead of failing the run. Interrupts (timeout, Stop) are uncatchable
// and still get through.
(function (host, maxChars) {
  'use strict';
  const apply = Reflect.apply;
  const ownKeys = Reflect.ownKeys;
  const getDesc = Object.getOwnPropertyDescriptor;
  const getProto = Object.getPrototypeOf;
  const hasOwn = Object.hasOwn;
  const isArray = Array.isArray;
  const ObjectProto = Object.prototype;
  const ErrorProto = Error.prototype;
  // quickjs-ng: `stack` is an accessor on Error.prototype.
  const stackDesc = getDesc(ErrorProto, 'stack');
  const stackGet = stackDesc !== undefined && typeof stackDesc.get === 'function' ? stackDesc.get : null;
  const isErrorStatic = typeof Error.isError === 'function' ? Error.isError : null;
  const StringC = String;
  const TypeErrorC = TypeError;
  const SyntaxErrorC = SyntaxError;
  const jsonStringify = JSON.stringify;
  const jsonParse = JSON.parse;
  const strSlice = String.prototype.slice;
  const strIndexOf = String.prototype.indexOf;
  const strCharCodeAt = String.prototype.charCodeAt;
  const strToWellFormed = typeof String.prototype.toWellFormed === 'function' ? String.prototype.toWellFormed : null;
  const fromCharCode = String.fromCharCode;
  const fnToString = Function.prototype.toString;
  const symToString = Symbol.prototype.toString;
  const mapSize = getDesc(Map.prototype, 'size').get;
  const setSize = getDesc(Set.prototype, 'size').get;
  const mapEntries = Map.prototype.entries;
  const setValues = Set.prototype.values;
  const mapIterNext = getProto(new Map().entries()).next;
  const setIterNext = getProto(new Set().values()).next;
  const dateGetTime = Date.prototype.getTime;
  const dateToISOString = Date.prototype.toISOString;
  const reSource = getDesc(RegExp.prototype, 'source').get;
  const reFlags = getDesc(RegExp.prototype, 'flags').get;
  const SetC = Set;
  const setAdd = Set.prototype.add;
  const setHas = Set.prototype.has;
  const setDelete = Set.prototype.delete;
  const emit = host.emit;
  const hostNote = host.note;

  const MAX_DEPTH = 8;
  const LINE_WIDTH = 72;
  let budget = 0;
  const ancestors = new SetC();

  const call = (fn, self, args) => apply(fn, self, args || []);
  const slice = (s, a, b) => call(strSlice, s, [a, b]);
  const contains = (s, sub) => call(strIndexOf, s, [sub]) !== -1;

  function brand(getter, x) {
    try {
      call(getter, x);
      return true;
    } catch {
      return false;
    }
  }

  // An own data property's value; undefined for accessors and missing keys.
  function ownData(o, key) {
    const d = getDesc(o, key);
    return d !== undefined && hasOwn(d, 'value') ? d.value : undefined;
  }

  // A data property's value found along the prototype chain (getters are skipped).
  function chainData(o, key) {
    for (let i = 0; o !== null && i < 64; i++) {
      const d = getDesc(o, key);
      if (d !== undefined) return hasOwn(d, 'value') ? d.value : undefined;
      o = getProto(o);
    }
    return undefined;
  }

  function isError(x) {
    if (isErrorStatic !== null) return isErrorStatic(x);
    for (let o = getProto(x), i = 0; o !== null && i < 64; o = getProto(o), i++) {
      if (o === ErrorProto) return true;
    }
    return false;
  }

  function isIdentifier(k) {
    if (k.length === 0) return false;
    for (let i = 0; i < k.length; i++) {
      const c = call(strCharCodeAt, k, [i]);
      const letter = (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c === 36 || c === 95;
      const digit = c >= 48 && c <= 57;
      if (!(letter || (digit && i > 0))) return false;
    }
    return true;
  }

  // At most `n` UTF-16 units of `s`, never ending between the halves of a surrogate pair.
  function cut(s, n) {
    if (s.length <= n) return s;
    let end = n > 0 ? n : 0;
    if (end > 0) {
      const c = call(strCharCodeAt, s, [end - 1]);
      if (c >= 0xd800 && c <= 0xdbff) end--;
    }
    return slice(s, 0, end);
  }

  // `s` with lone surrogates replaced by U+FFFD: the host can only take well-formed text.
  function wellFormed(s) {
    if (strToWellFormed !== null) return call(strToWellFormed, s);
    let out = '';
    for (let i = 0; i < s.length; i++) {
      const c = call(strCharCodeAt, s, [i]);
      if (c >= 0xd800 && c <= 0xdbff) {
        const d = i + 1 < s.length ? call(strCharCodeAt, s, [i + 1]) : 0;
        if (d >= 0xdc00 && d <= 0xdfff) {
          out += fromCharCode(c, d);
          i++;
          continue;
        }
        out += '\ufffd';
      } else if (c >= 0xdc00 && c <= 0xdfff) {
        out += '\ufffd';
      } else {
        out += fromCharCode(c);
      }
    }
    return out;
  }

  // A nested string, quoted. Cut to the budget first, so a huge string is never copied whole.
  function quote(s) {
    const room = budget > 0 ? budget : 0;
    return s.length > room ? jsonStringify(cut(s, room)) + '…' : jsonStringify(s);
  }

  function formatKey(k) {
    if (typeof k === 'symbol') return '[' + call(symToString, k) + ']';
    return isIdentifier(k) ? k : quote(k);
  }

  function errorText(e) {
    const name = chainData(e, 'name');
    const message = chainData(e, 'message');
    const n = typeof name === 'string' && name !== '' ? name : 'Error';
    return typeof message === 'string' && message !== '' ? n + ': ' + message : n;
  }

  function constructorName(proto) {
    const ctor = ownData(proto, 'constructor');
    if (typeof ctor !== 'function') return '';
    const name = ownData(ctor, 'name');
    return typeof name === 'string' ? name : '';
  }

  function formatFunction(f) {
    let name = '';
    let kind = 'Function';
    try {
      const n = ownData(f, 'name');
      if (typeof n === 'string') name = n;
      const src = call(fnToString, f);
      if (slice(src, 0, 6) === 'class ' || slice(src, 0, 6) === 'class{') {
        return name ? '[class ' + name + ']' : '[class (anonymous)]';
      }
      const proto = getProto(f);
      const ctorName = proto === null ? '' : constructorName(proto);
      if (ctorName !== '' && ctorName !== 'Function') kind = ctorName;
    } catch {
      // Proxy or revoked: keep the defaults.
    }
    return '[' + kind + (name ? ': ' + name : ' (anonymous)') + ']';
  }

  function indentOf(depth) {
    let s = '';
    for (let i = 0; i < depth; i++) s += '  ';
    return s;
  }

  // Joins formatted children on one line when short, otherwise one per line.
  function layout(prefix, open, close, one, multi, count, depth, pad) {
    if (count === 0) return prefix + open + close;
    const single = prefix + open + pad + one + pad + close;
    if (!contains(one, '\n') && single.length + depth * 2 <= LINE_WIDTH) return single;
    return prefix + open + multi + '\n' + indentOf(depth) + close;
  }

  // True for array index keys ("0", "1", …), which formatArray already showed.
  function isIndex(k) {
    if (typeof k !== 'string' || k === '') return false;
    const n = +k;
    return StringC(n >>> 0) === k && n >>> 0 !== 4294967295;
  }

  function formatProps(o, keys, depth, acc, skipIndices) {
    const childIndent = indentOf(depth + 1);
    for (let i = 0; i < keys.length; i++) {
      if (budget <= 0) {
        acc.one += (acc.count ? ', ' : '') + '…';
        acc.multi += (acc.count ? ',\n' : '\n') + childIndent + '…';
        acc.count++;
        return;
      }
      const k = keys[i];
      if (skipIndices && (k === 'length' || isIndex(k))) continue;
      let text;
      try {
        const d = getDesc(o, k);
        if (d === undefined || !d.enumerable) continue;
        let v;
        if (hasOwn(d, 'value')) {
          v = format(d.value, depth + 1);
        } else {
          const g = d.get !== undefined;
          const s = d.set !== undefined;
          v = g && s ? '[Getter/Setter]' : g ? '[Getter]' : '[Setter]';
        }
        text = formatKey(k) + ': ' + v;
      } catch {
        text = formatKey(k) + ': [unreadable]';
      }
      budget -= 2;
      acc.one += (acc.count ? ', ' : '') + text;
      acc.multi += (acc.count ? ',\n' : '\n') + childIndent + text;
      acc.count++;
    }
  }

  function add(acc, text, childIndent) {
    acc.one += (acc.count ? ', ' : '') + text;
    acc.multi += (acc.count ? ',\n' : '\n') + childIndent + text;
    acc.count++;
  }

  function formatArray(a, depth) {
    const acc = { one: '', multi: '', count: 0 };
    const childIndent = indentOf(depth + 1);
    const len = ownData(a, 'length');
    const n = typeof len === 'number' ? len : 0;
    for (let i = 0; i < n; i++) {
      if (budget <= 0) {
        add(acc, '… ' + (n - i) + ' more', childIndent);
        break;
      }
      const d = getDesc(a, StringC(i));
      let text;
      if (d === undefined) {
        text = '<empty>';
        budget -= 7;
      } else if (hasOwn(d, 'value')) {
        text = format(d.value, depth + 1);
      } else {
        text = '[Getter]';
      }
      budget -= 2;
      add(acc, text, childIndent);
    }
    // Non-index own keys (`a.total = 3`), after the elements.
    if (budget > 0) formatProps(a, ownKeys(a), depth, acc, true);
    const proto = getProto(a);
    const name = proto === null ? '' : constructorName(proto);
    const prefix = name !== '' && name !== 'Array' ? name + '(' + n + ') ' : '';
    return layout(prefix, '[', ']', acc.one, acc.multi, acc.count, depth, '');
  }

  function formatCollection(o, depth, isMap) {
    const acc = { one: '', multi: '', count: 0 };
    const childIndent = indentOf(depth + 1);
    const size = call(isMap ? mapSize : setSize, o);
    const it = call(isMap ? mapEntries : setValues, o);
    const next = isMap ? mapIterNext : setIterNext;
    for (;;) {
      if (budget <= 0) {
        add(acc, '…', childIndent);
        break;
      }
      const r = call(next, it);
      if (r.done) break;
      let text;
      if (isMap) {
        const entry = r.value;
        text = format(entry[0], depth + 1) + ' => ' + format(entry[1], depth + 1);
      } else {
        text = format(r.value, depth + 1);
      }
      budget -= 2;
      add(acc, text, childIndent);
    }
    const prefix = (isMap ? 'Map' : 'Set') + '(' + size + ') ';
    return layout(prefix, '{', '}', acc.one, acc.multi, acc.count, depth, ' ');
  }

  function formatObject(o, depth) {
    if (call(setHas, ancestors, [o])) return '[Circular]';
    if (brand(dateGetTime, o)) {
      try {
        return call(dateToISOString, o);
      } catch {
        return 'Invalid Date';
      }
    }
    if (brand(reSource, o)) return '/' + call(reSource, o) + '/' + call(reFlags, o);
    if (isError(o)) return '[' + errorText(o) + ']';
    const array = isArray(o);
    if (depth >= MAX_DEPTH) return array ? '[Array]' : '[Object]';
    call(setAdd, ancestors, [o]);
    try {
      if (array) return formatArray(o, depth);
      if (brand(mapSize, o)) return formatCollection(o, depth, true);
      if (brand(setSize, o)) return formatCollection(o, depth, false);
      const proto = getProto(o);
      let prefix = '';
      if (proto === null) {
        prefix = '[Object: null prototype] ';
      } else if (proto !== ObjectProto) {
        const name = constructorName(proto);
        if (name !== '' && name !== 'Object') prefix = name + ' ';
      }
      const acc = { one: '', multi: '', count: 0 };
      formatProps(o, ownKeys(o), depth, acc, false);
      return layout(prefix, '{', '}', acc.one, acc.multi, acc.count, depth, ' ');
    } finally {
      call(setDelete, ancestors, [o]);
    }
  }

  // One value, nested at `depth` (strings quoted).
  function format(v, depth) {
    let s;
    try {
      switch (typeof v) {
        case 'string':
          s = quote(v);
          break;
        case 'number':
          s = v === 0 && 1 / v < 0 ? '-0' : StringC(v);
          break;
        case 'bigint':
          s = StringC(v) + 'n';
          break;
        case 'boolean':
        case 'undefined':
          s = StringC(v);
          break;
        case 'symbol':
          s = call(symToString, v);
          break;
        case 'function':
          s = formatFunction(v);
          break;
        default:
          if (v === null) {
            s = 'null';
          } else {
            return formatObject(v, depth);
          }
      }
    } catch {
      return v === null ? 'null' : '[' + typeof v + ']';
    }
    budget -= s.length;
    return s;
  }

  // A top-level value: strings as they are, everything else formatted. Capped at `cap`
  // characters (default maxChars), with "…" marking the cut.
  function top(v, cap) {
    const limit = cap === undefined ? maxChars : cap;
    budget = limit;
    let s;
    try {
      s = typeof v === 'string' ? v : format(v, 0);
    } catch {
      s = v === null ? 'null' : '[' + typeof v + ']';
    }
    return wellFormed(s.length > limit ? cut(s, limit - 1) + '…' : s);
  }

  function stackOf(e) {
    let stack = ownData(e, 'stack');
    if (stack === undefined && stackGet !== null) {
      try {
        stack = call(stackGet, e);
      } catch {
        stack = undefined;
      }
    }
    return typeof stack === 'string' ? wellFormed(cut(stack, 4096)) : undefined;
  }

  // [text, stack] for the run's error: `Name: message` and the stack for errors, the
  // formatted value otherwise. Its own cap: it is shown even when the output is full.
  const ERROR_CHARS = 10000;
  function describeError(e) {
    try {
      if (e !== null && typeof e === 'object' && isError(e)) {
        let text = errorText(e);
        if (text.length > ERROR_CHARS) text = cut(text, ERROR_CHARS - 1) + '…';
        return [wellFormed(text), stackOf(e)];
      }
    } catch {
      // fall through to the plain format
    }
    return [top(e, ERROR_CHARS), undefined];
  }

  let full = false;
  function logger(level) {
    return function (...args) {
      if (full) return;
      let text = '';
      for (let i = 0; i < args.length; i++) {
        if (i > 0) text += ' ';
        text += top(args[i]);
        if (text.length > maxChars) break;
      }
      if (text.length > maxChars) text = cut(text, maxChars - 1) + '…';
      if (!emit(level, wellFormed(text))) full = true;
    };
  }

  function note(title) {
    if (typeof title !== 'string') {
      throw new TypeErrorC('note() expects a note title');
    }
    // The host returns the body behind a one-letter kind tag: "j" JSON, "m" markdown.
    const found = hostNote(title);
    const body = slice(found, 1);
    if (slice(found, 0, 1) === 'j') {
      try {
        return jsonParse(body);
      } catch {
        throw new SyntaxErrorC('Note "' + title + '" is not valid JSON');
      }
    }
    return body;
  }

  globalThis.console = {
    log: logger('log'),
    info: logger('info'),
    warn: logger('warn'),
    error: logger('error'),
    debug: logger('debug'),
  };
  globalThis.note = note;

  return { format: top, describeError };
})
