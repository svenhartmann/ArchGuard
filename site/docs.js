'use strict';
const docsSearch = document.getElementById('docs-search');
if (docsSearch) {
  const cards = [...document.querySelectorAll('[data-doc-card]')];
  docsSearch.addEventListener('input', () => {
    const terms = docsSearch.value.trim().toLocaleLowerCase('en').split(/\s+/).filter(Boolean);
    let count = 0;
    for (const card of cards) {
      const text = card.textContent.toLocaleLowerCase('en');
      card.hidden = !terms.every(term => text.includes(term));
      if (!card.hidden) count++;
    }
    document.getElementById('docs-search-status').textContent = `${count} ${count === 1 ? 'guide' : 'guides'}`;
    document.getElementById('docs-empty').hidden = count !== 0;
  });
}
