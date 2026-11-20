'use strict';
const docsSearch = document.getElementById('docs-search');
if (docsSearch) {
  const locale = document.documentElement.lang === 'de' ? 'de' : 'en';
  const cards = [...document.querySelectorAll('[data-doc-card]')];
  const filter = () => {
    const terms = docsSearch.value.trim().toLocaleLowerCase(locale).split(/\s+/).filter(Boolean);
    let count = 0;
    for (const card of cards) {
      const text = card.textContent.toLocaleLowerCase(locale);
      card.hidden = !terms.every(term => text.includes(term));
      if (!card.hidden) count++;
    }
    const label = locale === 'de' ? (count === 1 ? 'Leitfaden' : 'Leitfäden') : (count === 1 ? 'guide' : 'guides');
    document.getElementById('docs-search-status').textContent = `${count} ${label}`;
    document.getElementById('docs-empty').hidden = count !== 0;
  };
  docsSearch.addEventListener('input', filter);
  filter();
}
