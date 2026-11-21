'use strict';
const docsSearch = document.getElementById('docs-search');
const docsNavigation = document.querySelector('.docs-navigation');
if (docsNavigation && matchMedia('(max-width: 900px)').matches) docsNavigation.open = false;
if (docsSearch) {
  const german = document.documentElement.lang === 'de';
  const status = document.getElementById('docs-search-status');
  const results = document.getElementById('docs-search-results');
  const initialStatus = status.textContent;
  const normalize = value => value.toLocaleLowerCase(german ? 'de' : 'en').normalize('NFD').replace(/[\u0300-\u036f]/g, '').replace(/ß/g, 'ss');
  let indexPromise;
  let generation = 0;
  const loadIndex = () => {
    if (!indexPromise) {
      indexPromise = fetch(new URL('search-index.json', location.href)).then(response => {
        if (!response.ok) throw new Error('Search index unavailable');
        return response.json();
      }).catch(error => { indexPromise = undefined; throw error; });
    }
    return indexPromise;
  };
  const search = async () => {
    const current = ++generation;
    const query = docsSearch.value.trim();
    const terms = normalize(query).split(/\s+/).filter(Boolean);
    results.replaceChildren();
    results.hidden = true;
    if (!terms.length) { status.textContent = initialStatus; return; }
    status.textContent = german ? 'Suche läuft …' : 'Searching …';
    try {
      const records = await loadIndex();
      if (current !== generation) return;
      const matches = records.map(record => {
        const heading = normalize(record.title + ' ' + record.heading);
        const text = normalize(record.text);
        return {record, score: terms.every(term => (heading + ' ' + text).includes(term))
          ? 1 + terms.reduce((score, term) => score + (heading.includes(term) ? 5 : 0), 0) : 0};
      }).filter(item => item.score).sort((a, b) => b.score - a.score);
      for (const {record} of matches.slice(0, 20)) {
        const item = document.createElement('li');
        const link = document.createElement('a');
        link.href = record.url;
        link.textContent = record.heading === record.title ? record.title : `${record.title} · ${record.heading}`;
        const snippet = document.createElement('p');
        const text = record.text;
        const positions = terms.map(term => normalize(text).indexOf(term)).filter(pos => pos >= 0);
        const start = Math.max(0, (positions.length ? Math.min(...positions) : 0) - 60);
        snippet.textContent = (start ? '…' : '') + text.slice(start, start + 210) + (text.length > start + 210 ? '…' : '');
        item.append(link, snippet);
        results.append(item);
      }
      results.hidden = matches.length === 0;
      status.textContent = matches.length
        ? (german ? `${matches.length} Treffer` : `${matches.length} results`) + (matches.length > 20 ? (german ? ' · erste 20 angezeigt' : ' · showing first 20') : '')
        : (german ? 'Keine Treffer. Versuche einen anderen Begriff oder wähle ein Kapitel.' : 'No results. Try another term or choose a chapter.');
    } catch {
      if (current !== generation) return;
      status.textContent = german ? 'Suche gerade nicht verfügbar. Alle Kapitel sind über die Navigation erreichbar.' : 'Search is unavailable. All chapters remain available in the navigation.';
    }
  };
  docsSearch.addEventListener('input', search);
  docsSearch.addEventListener('keydown', event => {
    if (event.key === 'Escape') { docsSearch.value = ''; search(); }
  });
}
