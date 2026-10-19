// Shared ordered file list for the Zip and Merge tools.
function fileList(el, onChange) {
  const items = [];
  const render = () => {
    el.innerHTML = '';
    items.forEach((p, i) => {
      const row = document.createElement('div');
      row.className = 'item';
      row.innerHTML = `<span class="idx">${i + 1}</span><span class="name"></span>
        <button data-a="up" aria-label="Move up" ${i ? '' : 'disabled'}>↑</button>
        <button data-a="down" aria-label="Move down" ${i < items.length - 1 ? '' : 'disabled'}>↓</button>
        <button data-a="rm" aria-label="Remove">✕</button>`;
      row.querySelector('.name').textContent = p.split(/[\/]/).pop();
      row.querySelector('.name').title = p;
      row.dataset.i = i;
      el.append(row);
    });
    onChange(items);
  };
  el.addEventListener('click', e => {
    const b = e.target.closest('button'); if (!b) return;
    const i = +b.closest('.item').dataset.i;
    if (b.dataset.a === 'rm') items.splice(i, 1);
    else { const j = b.dataset.a === 'up' ? i - 1 : i + 1; [items[i], items[j]] = [items[j], items[i]]; }
    render();
  });
  return { items, add(paths) { items.push(...paths.filter(p => !items.includes(p))); render(); }, clear() { items.length = 0; render(); } };
}
