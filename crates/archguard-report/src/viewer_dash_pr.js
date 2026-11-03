'use strict';
// Review Dashboard PR comment panel (R42). Shows the last saved PR comment
// Markdown (R25): review_cmd.rs embeds the file the CLI or MCP wrote last as
// data.prComment; the register (R40) is rendered by the decisions panel,
// not here. Absence is stated with the exact regeneration command — an
// absent comment is never shown as an empty document. The preview honours a
// minimal safe Markdown subset and is built ONLY from
// createElement/createTextNode: the Markdown embeds project-derived,
// untrusted strings, so no HTML is ever assigned through innerHTML.

const DASH_PR_REGENERATE='archguard review <pfad> --format pr-comment --base <ref> --out';
const DASH_PR_FILE='.archguard/reports/pr-comment.md';

// Inline subset: `code` spans, **bold** inside the non-code parts. Every
// segment reaches the DOM as a text node — markers are split away, never
// replaced with markup.
function dashPrInline(text,parent){
  for(const [segment,isCode] of dashPrSplit(text,'`')){
    if(isCode){parent.appendChild(element('code',segment));continue;}
    for(const [chunk,isBold] of dashPrSplit(segment,'**')){
      if(isBold){const strong=document.createElement('strong');strong.appendChild(document.createTextNode(chunk));parent.appendChild(strong);}
      else parent.appendChild(document.createTextNode(chunk));
    }
  }
}
function dashPrSplit(text,marker){const parts=String(text).split(marker);return parts.map((part,index)=>[part,index%2===1]);}

function renderDashPr(container,kpis,register){
  dashClearPanel(container);
  const raw=data.prComment;
  if(typeof raw!=='string'||!raw.trim()){
    container.appendChild(element('p',t('No PR comment saved yet — nothing was generated for this review. Regenerate with "'+DASH_PR_REGENERATE+'" (CLI) or the pr_comment MCP tool; the file lives at '+DASH_PR_FILE+'.','Noch kein PR-Kommentar gespeichert — für dieses Review wurde nichts erzeugt. Mit „'+DASH_PR_REGENERATE+'" (CLI) oder dem MCP-Tool pr_comment neu erzeugen; die Datei liegt unter '+DASH_PR_FILE+'.'),'empty'));
    return;
  }
  // Toolbar: copy the RAW Markdown (not the rendered preview); the status
  // text is the visible feedback, including when the clipboard refuses.
  const bar=element('div',undefined,'dash-pr-bar');
  const button=element('button',t('Copy raw Markdown','Rohes Markdown kopieren'));
  button.type='button';
  const status=element('span',undefined,'muted dash-pr-status');
  button.addEventListener('click',()=>{
    const fail=()=>{
      status.textContent=t('Copy failed — select the raw Markdown below and copy it manually.','Kopieren fehlgeschlagen — das Roh-Markdown unten auswählen und manuell kopieren.');
      if(!bar.querySelector('textarea')){const area=element('textarea',undefined,'dash-pr-raw');area.readOnly=true;area.rows=8;area.value=raw;bar.appendChild(area);}
    };
    if(navigator.clipboard?.writeText)navigator.clipboard.writeText(raw).then(()=>{status.textContent=t('Raw Markdown copied to the clipboard.','Rohes Markdown in die Zwischenablage kopiert.');}).catch(fail);
    else fail();
  });
  bar.appendChild(button);bar.appendChild(status);
  container.appendChild(bar);
  // Line-based preview of the safe subset (headings, lists, rules, inline).
  const preview=element('div',undefined,'dash-pr-preview');
  let list=null;
  for(const line of raw.split(/\r?\n/)){
    if(line.startsWith('- ')){
      if(!list){list=element('ul');preview.appendChild(list);}
      const item=document.createElement('li');dashPrInline(line.slice(2),item);list.appendChild(item);
      continue;
    }
    list=null; // any other line ends an open list; a blank line also just separates
    if(!line.trim())continue;
    if(line.trim()==='---'){preview.appendChild(document.createElement('hr'));continue;}
    let handled=false;
    for(const [prefix,tag] of [['#### ','h5'],['### ','h4'],['## ','h3']]){
      if(line.startsWith(prefix)){const heading=document.createElement(tag);dashPrInline(line.slice(prefix.length),heading);preview.appendChild(heading);handled=true;break;}
    }
    if(handled)continue;
    const paragraph=document.createElement('p');dashPrInline(line,paragraph);preview.appendChild(paragraph);
  }
  container.appendChild(preview);
  container.appendChild(element('p',t('Last saved PR comment — regenerate via the CLI ("'+DASH_PR_REGENERATE+'") or the pr_comment MCP tool.','Zuletzt gespeicherter PR-Kommentar — über die CLI („'+DASH_PR_REGENERATE+'") oder das MCP-Tool pr_comment neu erzeugen.'),'muted dash-pr-provenance'));
}
