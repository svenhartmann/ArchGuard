'use strict';
// Native dialog supplies focus containment and Escape; image links still work without JS.
(() => {
 if (!window.HTMLDialogElement || !HTMLDialogElement.prototype.showModal) return;
 const de = document.documentElement.lang === 'de';
 const labels = de ? {close:'Schließen',zoom:'Originalgröße',fit:'Einpassen',original:'Original öffnen',title:'Bildansicht',error:'Das Bild konnte nicht geladen werden.'} : {close:'Close',zoom:'Actual size',fit:'Fit image',original:'Open original',title:'Image preview',error:'The image could not be loaded.'};
 const dialog = document.createElement('dialog');
 dialog.className = 'image-overlay';
 dialog.setAttribute('aria-label', labels.title);
 dialog.innerHTML = `<div class="image-overlay-toolbar"><span>${labels.title}</span><div><button type="button" data-zoom aria-pressed="false">${labels.zoom}</button><a data-original target="_blank" rel="noopener">${labels.original}</a><button type="button" data-close autofocus>${labels.close} <span aria-hidden="true">×</span></button></div></div><div class="image-overlay-stage"><img alt=""></div><p class="image-overlay-caption" id="image-overlay-caption"></p><p class="image-overlay-error" role="status"></p>`;
 dialog.setAttribute('aria-describedby', 'image-overlay-caption');
 document.body.append(dialog);
 const image = dialog.querySelector('img');
 const caption = dialog.querySelector('.image-overlay-caption');
 const error = dialog.querySelector('.image-overlay-error');
 const zoom = dialog.querySelector('[data-zoom]');
 let opener;
 const resetZoom = () => {dialog.classList.remove('is-zoomed');zoom.textContent=labels.zoom;zoom.setAttribute('aria-pressed','false');};
 image.addEventListener('error', () => {image.hidden=true;error.textContent=labels.error;});
 image.addEventListener('load', () => {image.hidden=false;error.textContent='';});
 dialog.querySelector('[data-close]').addEventListener('click', () => dialog.close());
 dialog.addEventListener('click', event => {if(event.target === dialog) dialog.close();});
 dialog.addEventListener('close', () => {document.documentElement.classList.remove('image-overlay-open');resetZoom();if(opener?.isConnected)opener.focus({preventScroll:true});});
 zoom.addEventListener('click', () => {const active=dialog.classList.toggle('is-zoomed');zoom.setAttribute('aria-pressed',String(active));zoom.textContent=active?labels.fit:labels.zoom;});
 // Make unlinked content images keyboard-accessible, retaining layout and alt text.
 for (const img of document.querySelectorAll('main img')) {
  if (img.closest('a,button')) continue;
  const button=document.createElement('button');button.type='button';button.className='image-preview-trigger';
  button.setAttribute('aria-label', `${labels.title}: ${img.alt}`);button.setAttribute('aria-haspopup','dialog');
  img.before(button);button.append(img);
 }
 for (const link of document.querySelectorAll('a[href]')) {
  if (/\.(png|jpe?g|webp|avif|gif)$/i.test(new URL(link.href,document.baseURI).pathname)) link.setAttribute('aria-haspopup','dialog');
 }
 document.addEventListener('click', event => {
  if(event.defaultPrevented || event.button!==0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const trigger=event.target.closest('a,.image-preview-trigger');
  if(!trigger || trigger.closest('dialog')) return;
  const thumbnail=trigger.querySelector('img');
  const url=new URL(trigger.getAttribute('href') || thumbnail?.currentSrc || thumbnail?.src || '',document.baseURI);
  if(!/\.(png|jpe?g|webp|avif|gif)$/i.test(url.pathname) || !['http:','https:','file:'].includes(url.protocol)) return;
  event.preventDefault();opener=trigger;resetZoom();error.textContent='';image.hidden=false;
  const figure=trigger.closest('figure');
  caption.textContent=figure?.querySelector('figcaption')?.textContent.trim() || thumbnail?.alt || trigger.textContent.trim() || labels.title;
  image.alt=thumbnail?.alt || caption.textContent;image.src=url.href;
  dialog.querySelector('[data-original]').href=url.href;
  dialog.showModal();document.documentElement.classList.add('image-overlay-open');
 });
})();
