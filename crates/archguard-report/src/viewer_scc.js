'use strict';
// Shared iterative Tarjan analysis for the main graph and snapshot comparisons.
function findSccs(v) {
  const nameOf = new Map(v.nodes.map(n => [n.id, n.name]));
  const adj = new Map(v.nodes.map(n => [n.id, []]));
  const self = new Set();
  for (const e of v.edges) {
    if (!adj.has(e.from) || !adj.has(e.to)) continue; // unresolvable skipped (scc.rs:52)
    adj.get(e.from).push(e.to);
    if (e.from === e.to) self.add(e.from);
  }
  const idx = new Map();
  const low = new Map();
  const onStack = new Set();
  const stack = [];
  let counter = 0;
  const result = [];
  for (const start of v.nodes.map(n => n.id)) {
    if (idx.has(start)) continue;
    const frames = [{ v: start, i: 0 }];
    idx.set(start, counter); low.set(start, counter); counter += 1;
    stack.push(start); onStack.add(start);
    while (frames.length) {
      const fr = frames[frames.length - 1];
      const succs = adj.get(fr.v);
      if (fr.i < succs.length) {
        const w = succs[fr.i];
        fr.i += 1;
        if (!idx.has(w)) {
          idx.set(w, counter); low.set(w, counter); counter += 1;
          stack.push(w); onStack.add(w);
          frames.push({ v: w, i: 0 });
        } else if (onStack.has(w)) {
          low.set(fr.v, Math.min(low.get(fr.v), idx.get(w)));
        }
      } else {
        frames.pop();
        if (frames.length) {
          const p = frames[frames.length - 1];
          low.set(p.v, Math.min(low.get(p.v), low.get(fr.v)));
        }
        if (low.get(fr.v) === idx.get(fr.v)) {
          const comp = [];
          for (;;) {
            const w = stack.pop();
            onStack.delete(w);
            comp.push(w);
            if (w === fr.v) break;
          }
          if (comp.length > 1 || comp.some(x => self.has(x))) {
            comp.sort((a, b) => (nameOf.get(a) < nameOf.get(b) ? -1
                                 : nameOf.get(a) > nameOf.get(b) ? 1 : a - b));
            result.push(comp);
          }
        }
      }
    }
  }
  const smallName = comp => nameOf.get(comp[0]);
  result.sort((a, b) => {
    const an = smallName(a);
    const bn = smallName(b);
    if (an !== bn) return an < bn ? -1 : 1;
    for (let i = 0; i < Math.min(a.length, b.length); i += 1) {
      if (a[i] !== b[i]) return a[i] - b[i];
    }
    return a.length - b.length;
  });
  return result.map((members, i) => ({ id: 'scc-' + (i + 1), members: members }));
}

if(typeof module!=='undefined')module.exports={findSccs};
