// LoreEq: the safe expression language of equation figures (docs/plot.md §10), DOM-free.
// The same language as lore-plot's Rust `equation::expr` (the server validates with that one);
// ui/plot/eqtests.json holds the cases both pass (tests/eqcore.test.js under node). No eval,
// no Function: names resolve only through Maps of declared variables, bound indices,
// constants and the functions below, so `constructor`, `__proto__` and friends never resolve.
//
//   LoreEq.parse(src, names)          → AST (throws Error with a message in Traditional Chinese)
//   LoreEq.evaluate(ast, env)         → number | number[]   (env: Map or plain object of values)
//   LoreEq.toLatex(ast, labels)       → LaTeX of the parsed tree (labels: name → LaTeX)
//   LoreEq.eq.*                       → helpers over a figure's `eq` (env, curve, partial, sensitivity, sweep)
(function (root) {
  "use strict";
  const MAX_LEN = 4000, MAX_DEPTH = 48, MAX_VEC = 4096, BUDGET = 2000000, MC_SAMPLES = 256, SEED = 0x5eed1234;
  // name → [min args, max args]
  const FUNCTIONS = new Map(Object.entries({
    log: [1, 1], ln: [1, 1], log2: [1, 1], log10: [1, 1], log1p: [1, 1], exp: [1, 1], sqrt: [1, 1], abs: [1, 1],
    sign: [1, 1], floor: [1, 1], ceil: [1, 1], round: [1, 1], sin: [1, 1], cos: [1, 1], tanh: [1, 1],
    sigmoid: [1, 1], softplus: [1, 1], relu: [1, 1], gelu: [1, 1], silu: [1, 1], erf: [1, 1], sg: [1, 1],
    ind: [1, 1], pow: [2, 2], clamp: [3, 3], if: [3, 3], piecewise: [3, Infinity],
    min: [1, Infinity], max: [1, Infinity], sum: [1, 1], prod: [1, 1], mean: [1, 1], len: [1, 1],
    argmax: [1, 1], argmin: [1, 1], dot: [2, 2], norm: [1, 1], norm1: [1, 1], cosine: [2, 2],
    softmax: [1, 1], logsumexp: [1, 1],
    ce: [1, 1], bce: [2, 2], focal_w: [2, 2], focal: [3, 3], dice: [2, 3], huber: [2, 2], mse: [1, 2],
    mae: [1, 2], kl_gauss: [4, 4], info_nce: [3, 3], triplet: [3, 3], smooth_ce: [3, 3], hinge: [1, 1],
  }));
  const BINDERS = new Set(["sum", "prod", "vec", "let", "E"]);
  const DISTS = new Map([["normal", 2], ["uniform", 2], ["bernoulli", 1], ["categorical", 1]]);
  const CONSTANTS = new Map([["pi", Math.PI], ["e", Math.E]]);
  const reserved = (n) => FUNCTIONS.has(n) || BINDERS.has(n) || CONSTANTS.has(n) || DISTS.has(n);
  const fail = (m) => { throw new Error(m); };

  // ── lexer ──
  const SYMS = ["**", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "^", "(", ")", "[", "]", ",", "<", ">", "!", "~"];
  function lex(src) {
    if ([...src].length > MAX_LEN) fail(`算式太長（最多 ${MAX_LEN} 字）`);
    const cs = [...src];
    const out = [];
    let i = 0;
    const digit = (c) => c >= "0" && c <= "9";
    const alpha = (c) => (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || c === "_";
    while (i < cs.length) {
      const c = cs[i];
      if (/\s/.test(c)) { i++; continue; }
      if (digit(c) || (c === "." && digit(cs[i + 1] ?? ""))) {
        const s = i;
        while (i < cs.length && (digit(cs[i]) || cs[i] === ".")) i++;
        if (cs[i] === "e" || cs[i] === "E") {
          let j = i + 1;
          if (cs[j] === "+" || cs[j] === "-") j++;
          if (digit(cs[j] ?? "")) { i = j; while (i < cs.length && digit(cs[i])) i++; }
        }
        const t = cs.slice(s, i).join("");
        const v = Number(t);
        if (!Number.isFinite(v) || (t.match(/\./g) ?? []).length > 1) fail(`第 ${s + 1} 字：數字「${t}」不對`);
        out.push({ at: s, num: v });
        continue;
      }
      if (alpha(c)) {
        const s = i;
        while (i < cs.length && (alpha(cs[i]) || digit(cs[i]))) i++;
        out.push({ at: s, name: cs.slice(s, i).join("") });
        continue;
      }
      const two = cs.slice(i, i + 2).join("");
      const sym = SYMS.find((x) => two.startsWith(x));
      if (!sym) fail(`第 ${i + 1} 字：不能用的字元「${c}」`);
      out.push({ at: i, sym: sym === "**" ? "^" : sym });
      i += sym.length;
    }
    return out;
  }

  // ── parser (AST: { t: "num"|"name"|"vec"|"index"|"neg"|"not"|"bin"|"call"|"loop"|"let"|"E", … }) ──
  function parse(src, names = []) {
    const known = new Set(names);
    const toks = lex(String(src ?? ""));
    let at = 0, depth = 0;
    const bound = [];
    const peek = () => toks[at];
    const pos = () => (toks[at] ? toks[at].at + 1 : toks.length ? toks.at(-1).at + 2 : 1);
    const is = (s) => toks[at]?.sym === s;
    const eat = (s) => (is(s) ? (at++, true) : false);
    const expect = (s) => { if (!eat(s)) fail(`第 ${pos()} 字：這裡要「${s}」`); };
    const enter = () => { if (++depth > MAX_DEPTH) fail("算式巢狀太深"); };
    const bin = (ops, next) => {
      let l = next();
      for (;;) {
        const op = ops.find((o) => is(o));
        if (!op) return l;
        at++;
        l = { t: "bin", op, a: l, b: next() };
      }
    };
    const or = () => { enter(); const r = bin(["||"], and); depth--; return r; };
    const and = () => bin(["&&"], cmp);
    const cmp = () => bin(["<=", ">=", "==", "!=", "<", ">"], add);
    const add = () => bin(["+", "-"], mul);
    const mul = () => bin(["*", "/"], unary);
    function unary() {
      enter();
      let r;
      if (eat("-")) r = { t: "neg", a: unary() };
      else if (eat("+")) r = unary();
      else if (eat("!")) r = { t: "not", a: unary() };
      else r = power();
      depth--;
      return r;
    }
    function power() {
      const base = postfix();
      if (eat("^")) return { t: "bin", op: "^", a: base, b: unary() };
      return base;
    }
    function postfix() {
      let n = atom();
      while (eat("[")) { const i = or(); expect("]"); n = { t: "index", a: n, i }; }
      return n;
    }
    function args() {
      const a = [];
      if (eat(")")) return a;
      for (;;) {
        a.push(or());
        if (eat(")")) return a;
        expect(",");
      }
    }
    function boundName(f) {
      const tk = peek();
      if (tk?.name && !reserved(tk.name)) { at++; return tk.name; }
      fail(`第 ${pos()} 字：${f}( 的第一個參數要是新的名稱`);
    }
    const withBound = (name, fn) => { bound.push(name); try { return fn(); } finally { bound.pop(); } };
    function atom() {
      const p = pos();
      const tk = toks[at];
      if (!tk) fail(`第 ${p} 字：算式沒寫完`);
      at++;
      if (tk.num !== undefined) return { t: "num", v: tk.num };
      if (tk.sym === "(") { const n = or(); expect(")"); return n; }
      if (tk.sym === "[") {
        enter();
        const items = [];
        if (!eat("]")) for (;;) { items.push(or()); if (eat("]")) break; expect(","); }
        depth--;
        if (items.length > MAX_VEC) fail("向量太長");
        return items.length === 1 ? items[0] : { t: "vec", items };
      }
      if (tk.name !== undefined) {
        const n = tk.name;
        if (eat("(")) { enter(); const r = call(n); depth--; return r; }
        if (bound.includes(n) || known.has(n) || CONSTANTS.has(n)) return { t: "name", n };
        fail(`不認得的名稱「${n}」`);
      }
      fail(`第 ${p} 字：這裡不能是「${tk.sym}」`);
    }
    function call(f) {
      const binder = BINDERS.has(f) && toks[at + 1]?.sym === ",";
      if (binder) {
        const name = boundName(f);
        expect(",");
        if (f === "let") {
          const v = or();
          expect(",");
          const body = withBound(name, or);
          expect(")");
          return { t: "let", n: name, v, body };
        }
        if (f === "E") {
          const d = peek()?.name;
          if (!d) fail(`第 ${pos()} 字：E( 的第二個參數要是分佈，例如 normal(0, 1)`);
          if (!DISTS.has(d)) fail(`不認得的函式「${d}」`);
          at++;
          expect("(");
          const a = args();
          if (a.length !== DISTS.get(d)) fail(`「${d}」的參數個數不對`);
          expect(",");
          const body = withBound(name, or);
          expect(")");
          return { t: "E", n: name, dist: d, args: a, body };
        }
        const lo = or();
        expect(",");
        const hi = or();
        expect(",");
        const body = withBound(name, or);
        expect(")");
        return { t: "loop", kind: f, n: name, lo, hi, body };
      }
      const a = args();
      const ar = FUNCTIONS.get(f);
      if (!ar) fail(`不認得的函式「${f}」`);
      if (a.length < ar[0] || a.length > ar[1] || (f === "piecewise" && a.length % 2 === 0)) fail(`「${f}」的參數個數不對`);
      return { t: "call", f, args: a };
    }
    const n = or();
    if (at < toks.length) fail(`第 ${toks[at].at + 1} 字：多出來的「${toks[at].sym ?? toks[at].name ?? toks[at].num}」`);
    return n;
  }

  /** Names an AST reads from outside. */
  function freeNames(ast) {
    const out = new Set();
    const walk = (n, b) => {
      switch (n.t) {
        case "name": if (!b.includes(n.n) && !CONSTANTS.has(n.n)) out.add(n.n); break;
        case "vec": n.items.forEach((x) => walk(x, b)); break;
        case "call": n.args.forEach((x) => walk(x, b)); break;
        case "index": walk(n.a, b); walk(n.i, b); break;
        case "bin": walk(n.a, b); walk(n.b, b); break;
        case "neg": case "not": walk(n.a, b); break;
        case "loop": walk(n.lo, b); walk(n.hi, b); walk(n.body, [...b, n.n]); break;
        case "let": walk(n.v, b); walk(n.body, [...b, n.n]); break;
        case "E": n.args.forEach((x) => walk(x, b)); walk(n.body, [...b, n.n]); break;
      }
    };
    walk(ast, []);
    return [...out];
  }

  // ── numbers ──
  /** erf, fractional error < 1.2e-7 (Numerical Recipes erfcc; the Rust side is the same formula). */
  function erf(x) {
    const z = Math.abs(x);
    const t = 1 / (1 + 0.5 * z);
    const ans = t * Math.exp(-z * z - 1.26551223 + t * (1.00002368 + t * (0.37409196 + t * (0.09678418 + t * (-0.18628806 + t * (0.27886807 + t * (-1.13520398 + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))));
    return x >= 0 ? 1 - ans : ans - 1;
  }
  const sigmoid = (x) => (x >= 0 ? 1 / (1 + Math.exp(-x)) : Math.exp(x) / (1 + Math.exp(x)));
  const softplus = (x) => Math.max(x, 0) + Math.log1p(Math.exp(-Math.abs(x)));
  function logsumexp(v) {
    const m = Math.max(...v);
    if (!Number.isFinite(m)) return m;
    return m + Math.log(v.reduce((s, x) => s + Math.exp(x - m), 0));
  }
  /** mulberry32 in u32 arithmetic (= Rust Rng). */
  function rng(seed) {
    let a = seed >>> 0;
    return {
      u32() {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return (t ^ (t >>> 14)) >>> 0;
      },
      uniform() { return (this.u32() + 0.5) / 4294967296; },
    };
  }

  /** Rust's f64::round: halves away from zero. */
  const rnd = (x) => Math.sign(x) * Math.round(Math.abs(x));
  const isVec = Array.isArray;
  const asVec = (v) => (isVec(v) ? v : [v]);
  const scalarOf = (v) => (isVec(v) ? (v.length === 1 ? v[0] : undefined) : v);
  const map1 = (v, f) => (isVec(v) ? v.map(f) : f(v));
  function zip2(a, b, f) {
    if (!isVec(a) && !isVec(b)) return f(a, b);
    if (!isVec(a)) return b.map((y) => f(a, y));
    if (!isVec(b)) return a.map((x) => f(x, b));
    if (a.length !== b.length) fail(`向量長度不同（${a.length} 與 ${b.length}）`);
    return a.map((x, i) => f(x, b[i]));
  }
  const truth = (x) => x !== 0 && !Number.isNaN(x);
  const B = (x) => (x ? 1 : 0);
  const BIN = {
    "+": (x, y) => x + y, "-": (x, y) => x - y, "*": (x, y) => x * y, "/": (x, y) => x / y, "^": (x, y) => Math.pow(x, y),
    "<": (x, y) => B(x < y), "<=": (x, y) => B(x <= y), ">": (x, y) => B(x > y), ">=": (x, y) => B(x >= y),
    "==": (x, y) => B(x === y), "!=": (x, y) => B(x !== y), "&&": (x, y) => B(truth(x) && truth(y)), "||": (x, y) => B(truth(x) || truth(y)),
  };
  const sumOf = (v) => v.reduce((s, x) => s + x, 0);

  function evaluate(ast, env) {
    const vars = env instanceof Map ? env : new Map(Object.entries(env ?? {}));
    const locals = [];
    let steps = 0, mcDepth = 0;
    const tick = (n) => { steps += n; if (steps > BUDGET) fail("計算量太大"); };
    const lookup = (n) => {
      for (let i = locals.length - 1; i >= 0; i--) if (locals[i][0] === n) return locals[i][1];
      if (vars.has(n)) return vars.get(n);
      if (CONSTANTS.has(n)) return CONSTANTS.get(n);
      fail(`不認得的名稱「${n}」`);
    };
    const scalar = (n) => { const v = scalarOf(ev(n)); if (v === undefined) fail("這裡要一個數，不是向量"); return v; };
    const int = (n) => { const v = scalar(n); if (!Number.isFinite(v)) fail("範圍不是有限的數"); return rnd(v); };
    function ev(n) {
      tick(1);
      switch (n.t) {
        case "num": return n.v;
        case "name": return lookup(n.n);
        case "vec": {
          const out = [];
          for (const it of n.items) out.push(...asVec(ev(it)));
          if (out.length > MAX_VEC) fail("向量太長");
          return out;
        }
        case "index": {
          const v = asVec(ev(n.a));
          const i = int(n.i);
          return i >= 1 && i <= v.length ? v[i - 1] : NaN;
        }
        case "neg": return map1(ev(n.a), (x) => -x);
        case "not": return map1(ev(n.a), (x) => B(!truth(x)));
        case "bin": { const a = ev(n.a), b = ev(n.b); return zip2(a, b, BIN[n.op]); }
        case "loop": {
          const lo = int(n.lo), hi = int(n.hi);
          const count = Math.max(0, hi - lo + 1);
          if (count > MAX_VEC) fail(`索引範圍太大（${count}，最多 ${MAX_VEC}）`);
          tick(count);
          let acc, items = [];
          for (let i = lo; i <= hi; i++) {
            locals.push([n.n, i]);
            let v;
            try { v = ev(n.body); } finally { locals.pop(); }
            if (n.kind === "vec") { items.push(...asVec(v)); if (items.length > MAX_VEC) fail("向量太長"); }
            else if (n.kind === "sum") acc = acc === undefined ? v : zip2(acc, v, BIN["+"]);
            else acc = acc === undefined ? v : zip2(acc, v, BIN["*"]);
          }
          return n.kind === "vec" ? items : acc ?? (n.kind === "sum" ? 0 : 1);
        }
        case "let": {
          const v = ev(n.v);
          locals.push([n.n, v]);
          try { return ev(n.body); } finally { locals.pop(); }
        }
        case "E": {
          const a = n.args.map(ev);
          mcDepth++;
          const r = rng((SEED + mcDepth * 7919) >>> 0);
          let acc;
          try {
            for (let k = 0; k < MC_SAMPLES; k++) {
              const s = sample(n.dist, a, r);
              locals.push([n.n, s]);
              let v;
              try { v = ev(n.body); } finally { locals.pop(); }
              acc = acc === undefined ? v : zip2(acc, v, BIN["+"]);
            }
          } finally { mcDepth--; }
          return map1(acc ?? NaN, (x) => x / MC_SAMPLES);
        }
        case "call": return call(n.f, n.args);
      }
      fail("壞掉的算式");
    }
    function call(f, args) {
      if (f === "if") return ev(args[truth(scalar(args[0])) ? 1 : 2]);
      if (f === "piecewise") {
        for (let i = 0; i + 1 < args.length; i += 2) if (truth(scalar(args[i]))) return ev(args[i + 1]);
        return ev(args.at(-1));
      }
      const a = args.map(ev);
      tick(a.reduce((s, v) => s + (isVec(v) ? v.length : 1), 0));
      const s = (i) => { const v = scalarOf(a[i]); if (v === undefined) fail(`${f}( 的第 ${i + 1} 個參數要是一個數`); return v; };
      const one = a[0];
      switch (f) {
        case "log": case "ln": return map1(one, Math.log);
        case "log2": return map1(one, Math.log2);
        case "log10": return map1(one, Math.log10);
        case "log1p": return map1(one, Math.log1p);
        case "exp": return map1(one, Math.exp);
        case "sqrt": return map1(one, Math.sqrt);
        case "abs": return map1(one, Math.abs);
        case "sign": return map1(one, (x) => (x > 0 ? 1 : x < 0 ? -1 : 0));
        case "floor": return map1(one, Math.floor);
        case "ceil": return map1(one, Math.ceil);
        // Rust's round: halves away from zero
        case "round": return map1(one, rnd);
        case "sin": return map1(one, Math.sin);
        case "cos": return map1(one, Math.cos);
        case "tanh": return map1(one, Math.tanh);
        case "sigmoid": return map1(one, sigmoid);
        case "softplus": return map1(one, softplus);
        case "relu": return map1(one, (x) => Math.max(x, 0));
        case "gelu": return map1(one, (x) => 0.5 * x * (1 + erf(x / Math.SQRT2)));
        case "silu": return map1(one, (x) => x * sigmoid(x));
        case "erf": return map1(one, erf);
        case "sg": return one;
        case "ind": return map1(one, (x) => B(truth(x)));
        case "hinge": return map1(one, (x) => Math.max(1 - x, 0));
        case "pow": return zip2(a[0], a[1], Math.pow);
        case "clamp": { const lo = s(1), hi = s(2); return map1(one, (x) => Math.min(Math.max(x, lo), hi)); }
        case "min": case "max": {
          const all = a.flatMap(asVec);
          if (!all.length) fail(`${f}( 沒有東西`);
          if (all.some(Number.isNaN)) return NaN;
          return f === "min" ? Math.min(...all) : Math.max(...all);
        }
        case "sum": return sumOf(asVec(one));
        case "prod": return asVec(one).reduce((p, x) => p * x, 1);
        case "mean": { const v = asVec(one); return sumOf(v) / v.length; }
        case "len": return asVec(one).length;
        case "argmax": case "argmin": {
          const v = asVec(one);
          let b = 0;
          v.forEach((x, i) => { if ((f === "argmax" && x > v[b]) || (f === "argmin" && x < v[b])) b = i; });
          return b + 1;
        }
        case "dot": return sumOf(asVec(zip2(a[0], a[1], BIN["*"])));
        case "norm": return Math.sqrt(sumOf(asVec(one).map((x) => x * x)));
        case "norm1": return sumOf(asVec(one).map(Math.abs));
        case "cosine": {
          const u = asVec(a[0]), v = asVec(a[1]);
          const d = sumOf(asVec(zip2(u, v, BIN["*"])));
          return d / (Math.sqrt(sumOf(u.map((x) => x * x))) * Math.sqrt(sumOf(v.map((x) => x * x))));
        }
        case "softmax": { const v = asVec(one), l = logsumexp(v); return v.map((x) => Math.exp(x - l)); }
        case "logsumexp": return logsumexp(asVec(one));
        case "ce": return map1(one, (p) => -Math.log(p));
        case "bce": return zip2(a[0], a[1], (p, y) => -(y * Math.log(p) + (1 - y) * Math.log(1 - p)));
        case "focal_w": return zip2(a[0], a[1], (p, g) => Math.pow(1 - p, g));
        case "focal": { const g = s(1), al = s(2); return map1(one, (p) => -al * Math.pow(1 - p, g) * Math.log(p)); }
        case "dice": {
          const sm = a.length > 2 ? s(2) : 0;
          const pg = sumOf(asVec(zip2(a[0], a[1], BIN["*"])));
          return 1 - (2 * pg + sm) / (sumOf(asVec(a[0])) + sumOf(asVec(a[1])) + sm);
        }
        case "huber": { const d = s(1); return map1(one, (r) => (Math.abs(r) <= d ? 0.5 * r * r : d * (Math.abs(r) - 0.5 * d))); }
        case "mse": case "mae": {
          const r = asVec(a.length === 2 ? zip2(a[0], a[1], BIN["-"]) : one);
          return sumOf(r.map((x) => (f === "mse" ? x * x : Math.abs(x)))) / r.length;
        }
        case "kl_gauss": { const m1 = s(0), s1 = s(1), m2 = s(2), s2 = s(3); return Math.log(s2 / s1) + (s1 * s1 + (m1 - m2) * (m1 - m2)) / (2 * s2 * s2) - 0.5; }
        case "info_nce": { const sp = s(0), tau = s(2); return logsumexp([sp / tau, ...asVec(a[1]).map((x) => x / tau)]) - sp / tau; }
        case "triplet": return Math.max(s(0) - s(1) + s(2), 0);
        case "smooth_ce": {
          const p = s(0), eps = s(1), k = s(2);
          return -(1 - eps + eps / k) * Math.log(p) - (k - 1) * (eps / k) * Math.log((1 - p) / (k - 1));
        }
      }
      fail(`不認得的函式「${f}」`);
    }
    function sample(dist, a, r) {
      const s = (i) => { const v = scalarOf(a[i]); if (v === undefined) fail(`${dist}( 的參數要是數`); return v; };
      if (dist === "normal") { const u1 = r.uniform(), u2 = r.uniform(); return s(0) + s(1) * Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2); }
      if (dist === "uniform") { const u = r.uniform(); return s(0) + (s(1) - s(0)) * u; }
      if (dist === "bernoulli") return B(r.uniform() < s(0));
      const p = asVec(a[0]);
      let u = r.uniform() * sumOf(p);
      for (let i = 0; i < p.length; i++) { if (u < p[i]) return i + 1; u -= p[i]; }
      return p.length;
    }
    return ev(ast);
  }

  // ── LaTeX of the parsed tree (what the program will compute, beside the paper's LaTeX) ──
  const GREEK = new Set(["alpha", "beta", "gamma", "delta", "epsilon", "varepsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu", "xi", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega", "Gamma", "Delta", "Theta", "Lambda", "Xi", "Pi", "Sigma", "Phi", "Psi", "Omega"]);
  function nameTex(n, labels) {
    const l = labels?.[n];
    if (l) return String(l).replace(/^\$|\$$/g, "").replace(/（.*?）|\(.*?\)$/g, "").trim();
    if (n === "eps") return "\\varepsilon";
    const m = /^([A-Za-z]+?)(\d+|_[A-Za-z0-9]+)?$/.exec(n);
    if (m) {
      const base = GREEK.has(m[1]) ? `\\${m[1]}` : m[1].length > 1 ? `\\mathit{${m[1]}}` : m[1];
      const sub = m[2] ? `_{${m[2].replace(/^_/, "")}}` : "";
      return base + sub;
    }
    return `\\mathit{${n.replace(/_/g, "\\_")}}`;
  }
  const PREC = { "||": 1, "&&": 2, "<": 3, "<=": 3, ">": 3, ">=": 3, "==": 3, "!=": 3, "+": 4, "-": 4, "*": 5, "/": 5, "^": 7 };
  const OPTEX = { "<": "<", "<=": "\\le", ">": ">", ">=": "\\ge", "==": "=", "!=": "\\ne", "&&": "\\wedge", "||": "\\vee", "+": "+", "-": "-" };
  const FTEX = { log: "\\log", ln: "\\ln", exp: "\\exp", sin: "\\sin", cos: "\\cos", tanh: "\\tanh", sigmoid: "\\sigma", min: "\\min", max: "\\max", log2: "\\log_2", log10: "\\log_{10}" };
  function fmtNum(v) {
    if (Number.isInteger(v)) return String(v);
    const s = String(+v.toPrecision(6));
    return /e/.test(s) ? s.replace(/e([+-]?\d+)/, (_, e) => `\\times 10^{${+e}}`) : s;
  }
  /** `colors`: name → CSS colour; each variable is wrapped in \\textcolor (3Blue1Brown-style colour-coding). */
  function toLatex(ast, labels = {}, colors = {}) {
    const P = (n) => (n.t === "bin" ? PREC[n.op] : n.t === "neg" ? 6 : 9);
    const wrap = (s) => `\\left(${s}\\right)`;
    const L = (n, ctx = 0) => {
      let s;
      switch (n.t) {
        case "num": s = fmtNum(n.v); break;
        case "name": s = colors[n.n] ? `\\textcolor{${colors[n.n]}}{${nameTex(n.n, labels)}}` : nameTex(n.n, labels); break;
        case "vec": s = `\\left[${n.items.map((x) => L(x)).join(",\\ ")}\\right]`; break;
        case "index": s = `${L(n.a, 9)}_{${L(n.i)}}`; break;
        case "neg": s = `-${L(n.a, 6)}`; break;
        case "not": s = `\\neg ${L(n.a, 6)}`; break;
        case "bin":
          if (n.op === "/") s = `\\frac{${L(n.a)}}{${L(n.b)}}`;
          else if (n.op === "^") s = `{${L(n.a, 8)}}^{${L(n.b)}}`;
          else if (n.op === "*") {
            // a number on the right needs a visible product sign; everything else a thin space
            const dot = n.b.t === "num" || n.b.t === "neg" || (n.b.t === "bin" && n.b.op === "^" && n.b.a.t === "num");
            s = `${L(n.a, 5)}${dot ? " \\cdot " : "\\,"}${L(n.b, 5.5)}`;
          } else s = `${L(n.a, PREC[n.op])} ${OPTEX[n.op]} ${L(n.b, PREC[n.op] + 0.5)}`;
          break;
        case "loop": {
          const lim = `_{${nameTex(n.n)}=${L(n.lo)}}^{${L(n.hi)}}`;
          if (n.kind === "vec") s = `\\left[${L(n.body)}\\right]${lim}`;
          else s = `${n.kind === "sum" ? "\\sum" : "\\prod"}${lim} ${L(n.body, 5)}`;
          break;
        }
        case "let": s = `${L(n.body)},\\quad ${nameTex(n.n)} = ${L(n.v)}`; break;
        case "E": {
          const d = { normal: (a) => `\\mathcal{N}\\left(${L(a[0])},\\,${L(a[1], 7)}^{2}\\right)`, uniform: (a) => `\\mathcal{U}\\left(${L(a[0])},\\,${L(a[1])}\\right)`, bernoulli: (a) => `\\mathrm{Bern}\\left(${L(a[0])}\\right)`, categorical: (a) => `\\mathrm{Cat}\\left(${L(a[0])}\\right)` }[n.dist](n.args);
          s = `\\mathbb{E}_{${nameTex(n.n)} \\sim ${d}}\\left[${L(n.body)}\\right]`;
          break;
        }
        case "call": {
          const a = n.args;
          const A = (i, c = 0) => L(a[i], c);
          switch (n.f) {
            case "sqrt": s = `\\sqrt{${A(0)}}`; break;
            case "abs": s = `\\left|${A(0)}\\right|`; break;
            case "norm": s = `\\left\\lVert ${A(0)} \\right\\rVert_2`; break;
            case "norm1": s = `\\left\\lVert ${A(0)} \\right\\rVert_1`; break;
            case "exp": s = `e^{${A(0)}}`; break;
            case "pow": s = `{${A(0, 8)}}^{${A(1)}}`; break;
            case "sg": s = `\\mathrm{sg}\\left[${A(0)}\\right]`; break;
            case "ind": s = `\\mathbb{1}\\left[${A(0)}\\right]`; break;
            case "ce": s = `-\\log ${A(0, 9)}`; break;
            case "focal_w": s = `{\\left(1 - ${A(0)}\\right)}^{${A(1)}}`; break;
            case "focal": s = `-${A(2, 5)}\\,{\\left(1 - ${A(0)}\\right)}^{${A(1)}}\\log ${A(0, 9)}`; break;
            case "bce": s = `-\\left[${A(1, 5)}\\log ${A(0, 9)} + \\left(1 - ${A(1)}\\right)\\log\\left(1 - ${A(0)}\\right)\\right]`; break;
            case "dot": s = `${A(0, 9)}^{\\top} ${A(1, 9)}`; break;
            case "cosine": s = `\\cos\\left(${A(0)},\\,${A(1)}\\right)`; break;
            case "clamp": s = `\\operatorname{clip}\\left(${A(0)},\\,${A(1)},\\,${A(2)}\\right)`; break;
            case "if": s = `\\begin{cases} ${A(1)} & ${A(0)} \\\\ ${A(2)} & \\text{otherwise} \\end{cases}`; break;
            case "piecewise": {
              const rows = [];
              for (let i = 0; i + 1 < a.length; i += 2) rows.push(`${A(i + 1)} & ${A(i)}`);
              rows.push(`${A(a.length - 1)} & \\text{otherwise}`);
              s = `\\begin{cases} ${rows.join(" \\\\ ")} \\end{cases}`;
              break;
            }
            case "sum": s = `\\sum ${A(0, 9)}`; break;
            case "mean": s = `\\overline{${A(0)}}`; break;
            default: {
              const head = FTEX[n.f] ?? `\\operatorname{${n.f.replace(/_/g, "\\_")}}`;
              s = `${head}\\left(${a.map((x) => L(x)).join(",\\,")}\\right)`;
            }
          }
          break;
        }
        default: s = "?";
      }
      return P(n) < ctx ? wrap(s) : s;
    };
    return L(ast);
  }

  /** A symbol's spellings in LaTeX: `p_t` ⇄ `p_{t}`, spaces dropped. */
  function spellings(tex) {
    const t = String(tex ?? "").replace(/\s+/g, "");
    if (!t) return [];
    const a = t.replace(/_\{([^{}])\}/g, "_$1"), b = t.replace(/_([A-Za-z0-9])(?![A-Za-z0-9}])/g, "_{$1}");
    return [...new Set([t, a, b])];
  }
  /**
   * Colour each listed symbol inside someone else's LaTeX (a paper's equation): `symbols` = [{ tex, color }].
   * Longest spellings first; a match never continues into a longer command (\\gamma ≠ \\gammaX).
   */
  function colorTex(latex, symbols) {
    let s = String(latex ?? "").replace(/\s+(?=[_^{}])/g, "");
    const list = symbols.flatMap((x) => spellings(x.tex).map((sp) => ({ sp, color: x.color }))).filter((x) => x.sp && x.color).sort((a, b) => b.sp.length - a.sp.length);
    const keep = [];
    for (const { sp, color } of list) {
      const esc = sp.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const re = new RegExp(`${esc}(?![A-Za-z])`, "g");
      s = s.replace(re, () => { keep.push(`\\textcolor{${color}}{${sp}}`); return `\u0001${keep.length - 1}\u0002`; });
    }
    return s.replace(/\u0001(\d+)\u0002/g, (_, k) => keep[+k]);
  }

  // ── an equation figure's `eq` ──
  const num = (v) => (typeof v === "number" ? v : Number(v));
  const EQ = {
    bars: (eq) => eq?.output === "bars",
    names(eq) {
      const v = (eq.vars ?? []).map((x) => x.name);
      if (eq.x && !EQ.bars(eq)) v.push(eq.x.name);
      return v;
    },
    param: (eq, name) => (eq.vars ?? []).find((v) => v.name === name),
    compile(eq, src) { return parse(src ?? eq.expr, EQ.names(eq)); },
    /** The variables (a Map) at their current values, `over` ({name: value}) on top. */
    env(eq, over = {}) {
      const m = new Map();
      for (const v of eq.vars ?? []) {
        if (Array.isArray(v.value)) m.set(v.name, v.value.map(num));
        else { const x = num(v.value); m.set(v.name, v.integer ? rnd(x) : x); }
      }
      if (eq.x && !EQ.bars(eq)) m.set(eq.x.name, num(eq.x.min ?? 0));
      for (const [k, x] of Object.entries(over)) m.set(k, EQ.param(eq, k)?.integer ? rnd(x) : x);
      return m;
    },
    xs(eq, n = 201) {
      const a = num(eq.x?.min ?? 0), b = num(eq.x?.max ?? 1);
      return Array.from({ length: n }, (_, i) => a + ((b - a) * i) / Math.max(1, n - 1));
    },
    value(eq, ast, over) {
      const v = scalarOf(evaluate(ast, EQ.env(eq, over)));
      return Number.isFinite(v) ? v : null;
    },
    curve(eq, ast, xs, over = {}) {
      const env = EQ.env(eq, over);
      const xn = eq.x.name;
      return xs.map((x) => { env.set(xn, x); const v = scalarOf(evaluate(ast, env)); return Number.isFinite(v) ? v : null; });
    },
    /** ∂f/∂name at the point (central difference inside the range; the Rust side is the same). */
    partial(eq, ast, name, over = {}) {
      const env = EQ.env(eq, over);
      const at = env.get(name);
      if (typeof at !== "number") return null;
      const rv = eq.x?.name === name ? eq.x : EQ.param(eq, name);
      const lo = num(rv?.min ?? at - 1), hi = num(rv?.max ?? at + 1);
      const h = Math.max(Math.abs(hi - lo) * 1e-4, 1e-9);
      const a = Math.max(at - h, lo), b = Math.min(at + h, hi);
      if (!(b > a)) return null;
      const fa = EQ.value(eq, ast, { ...over, [name]: a }), fb = EQ.value(eq, ast, { ...over, [name]: b });
      return fa == null || fb == null ? null : (fb - fa) / (b - a);
    },
    /** 敏感度: each slider's ∂f/∂θ × its range, normalised so the largest is ±1. */
    sensitivity(eq, ast, x0) {
      const over = eq.x && !EQ.bars(eq) ? { [eq.x.name]: x0 } : {};
      const out = (eq.vars ?? []).filter((v) => v.role === "param").map((p) => {
        const d = EQ.partial(eq, ast, p.name, over) ?? 0;
        return { name: p.name, label: p.label, d, scaled: d * (num(p.max) - num(p.min)) };
      });
      const m = Math.max(0, ...out.map((s) => Math.abs(s.scaled)));
      for (const s of out) s.norm = m > 0 ? s.scaled / m : 0;
      return out;
    },
    sweep(v, n) {
      const a = num(v.min), b = num(v.max);
      n = Math.max(2, n);
      return Array.from({ length: n }, (_, i) => {
        const t = i / (n - 1);
        const x = v.log && a > 0 && b > 0 ? Math.exp(Math.log(a) + t * (Math.log(b) - Math.log(a))) : a + t * (b - a);
        return v.integer ? rnd(x) : x;
      });
    },
    /** Ping-pong values for a GIF: n frames, each end once (= Rust sweep_frames). */
    sweepFrames(v, n) {
      n = Math.max(2, n);
      const up = Math.floor(n / 2) + 1;
      const vals = EQ.sweep(v, up);
      return [...vals, ...vals.slice(1, -1).reverse().slice(0, n - up)];
    },
  };

  const LoreEq = { parse, evaluate, freeNames, toLatex, colorTex, spellings, erf, sigmoid, softplus, rng, FUNCTIONS: [...FUNCTIONS.keys()], eq: EQ, MC_SAMPLES };
  if (typeof module !== "undefined" && module.exports) module.exports = LoreEq;
  else root.LoreEq = LoreEq;
})(typeof globalThis !== "undefined" ? globalThis : this);
