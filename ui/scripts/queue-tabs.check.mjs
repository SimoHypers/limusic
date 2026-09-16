// Regression: showing history, visiting Lyrics and returning to Queue must retain the viewport.
// Run: node ui/scripts/queue-tabs.check.mjs
// Uses real NowPlaying, QueueList, TrackRow, Tabs, shortcuts, appearance, grouping and virtualization; playback,
// track menus, lyrics and backend calls are mocked. Does not exercise native playback/WebView2.
// Requires optional Playwright tooling. Generated fixtures/results live under gitignored ui/perf.
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
// Playwright is optional test tooling; install it separately or set PLAYWRIGHT_MODULE to its file URL.
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
const root = fileURLToPath(new URL('../perf/queue-tabs/', import.meta.url));
const fixtures = {
  "player.svelte.ts": "export const np = $state({ open: true, tab: 'queue' });\nexport const ui = $state({ sidebarCollapsed: true });\nexport const playback = $state({ now: null, paused: true, volume: 50, queue: { items: Array.from({length: 30}, (_, i) => ({ video_id: `track-${i}`, title: `Track ${i + 1}`, artists: 'Fixture artist' })), currentIndex: 8, playedFrom: 0, sourceName: 'Test playlist' } });\nexport function wheelVolume() {}\nexport function openAddToPlaylist() {}\nexport const toast = { error: (message) => { (window.historySaveErrors ??= []).push(message); } };\r\n",
  "video.svelte.ts": "export const canVideo = () => false;\nexport const showVideo = () => false;\nexport const claimVideo = () => {};\nexport const parkVideo = () => {};\nexport const video = $state({want: false});\r\n",
  "api.ts": "export const allowFontFile = async () => {};\nexport const togglePause = () => {};\nexport const moveInQueue = () => {};\nexport const playIndex = () => {};\nexport const removeFromQueue = () => {};\nexport const clearQueued = () => {};\r\n",
  "environment.ts": "export const beforeNavigate = () => {};\nexport const browser = true;\r\n",
  "LyricsView.svelte": "<p>Lyrics fixture</p>\r\n",
  "App.svelte": "<script>\nimport NowPlaying from '../../src/lib/components/NowPlaying.svelte';\nimport { appearance, initTheme, setAppearance } from '../../src/lib/theme.svelte';\ninitTheme();\nwindow.setHistoryVisible = (visible) => setAppearance({queueHistoryVisible:visible});\n</script>\n<output id=\"preference\" data-visible={appearance.queueHistoryVisible}></output>\n<NowPlaying queueOpen={false} lyricsOpen={false} />\r\n",
  "main.js": "import { mount } from 'svelte';\nimport './fixture.css';\nimport App from './App.svelte';\nmount(App, { target: document.getElementById('app') });\r\n",
  "index.html": "<div id=\"app\"></div><script type=\"module\" src=\"/main.js\"></script>\r\n",
  "fixture.css": "@import '../../src/routes/layout.css';\n@source '../../src';\n@source '../../node_modules/bits-ui/dist';\r\n"
};
fixtures['player.svelte.ts'] += '\nexport const cycleRepeat = () => {};\nexport const nudgeVolume = () => {};\nexport const toggleMute = () => {};\n';
fixtures['api.ts'] = fixtures['api.ts'].replace('export const togglePause = () => {};',
  'export const togglePause = () => { window.pauseCalls = (window.pauseCalls || 0) + 1; };');
fixtures['App.svelte'] = fixtures['App.svelte'].replace('initTheme();',
  "import { onMount } from 'svelte';\nimport { initShortcuts } from '../../src/lib/shortcuts';\nonMount(initShortcuts);\ninitTheme();");
fixtures['player.svelte.ts'] += '\nexport const anySaved = () => false; export const isLiked = () => false; export const ratingOf = () => \'indifferent\'; export const savedPlaylists = () => []; export const toggleRating = () => {};';
fixtures['environment.ts'] += '\nexport const goto = () => {};';
fixtures['Menu.svelte'] = '<button style="width:28px;height:28px" aria-label="Track menu fixture"></button>';
fixtures['App.svelte'] = fixtures['App.svelte']
  .replace("import NowPlaying", "import QueuePanel from '../../src/lib/components/QueuePanel.svelte';\nimport NowPlaying")
  .replace('<NowPlaying queueOpen={false} lyricsOpen={false} />',
    '{#if location.search.includes(\'panel\')}<QueuePanel onClose={() => {}} />{:else}<NowPlaying queueOpen={false} lyricsOpen={false} />{/if}');
mkdirSync(root, { recursive: true });
for (const [name, source] of Object.entries(fixtures)) writeFileSync(resolve(root, name), source);
const lib = resolve(root, '../../src/lib');
const mock = (name) => resolve(root, name);
const server = await createServer({ configFile: false, root, server: {host:'127.0.0.1',port:5197,strictPort:true,fs:{allow:[resolve(root,'../..')]}},
 plugins: [{name:'fixture-imports',enforce:'pre',resolveId(id){
 if (/\/api(?:\.ts)?$/.test(id) || id === './api') return mock('api.ts');
 if (/\/player\.svelte(?:\.ts)?$/.test(id)) return mock('player.svelte.ts');
 if (/\/video\.svelte(?:\.ts)?$/.test(id)) return mock('video.svelte.ts');
 if (id === './TrackMenu.svelte' || id === './SavedInPlaylists.svelte') return mock('Menu.svelte');
 if (id === './LyricsView.svelte') return mock('LyricsView.svelte');
 if (id.startsWith('$app/')) return mock('environment.ts');
 }},tailwindcss(),svelte({configFile:false,compilerOptions:{runes:true}})],
 resolve: { alias: {'$lib':lib} } });
await server.listen();
let browser;
try {
 browser = await chromium.launch({headless:true, executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined});
 const page = await browser.newPage({viewport:{width:1100,height:800}});
 const pageErrors=[];
 page.on('pageerror',e=>pageErrors.push(e.message));
 await page.goto('http://127.0.0.1:5197');
 await page.getByRole('button',{name:'Show history'}).waitFor();
 const settled = () => page.waitForFunction(() => !document.querySelector('[data-history-transitioning]'));
 const state = async (label) => {
   await settled();
   const data = await page.evaluate(() => { const btn=document.querySelector('button[aria-expanded]'); const heading=btn && document.getElementById(btn.getAttribute('aria-controls')); const scroller=document.getElementById(btn.getAttribute('aria-controls'))?.parentElement; return {preference:document.querySelector('#preference').getAttribute('data-visible'),button:btn?.textContent.trim(),expanded:btn?.getAttribute('aria-expanded'),historyInDom:!!heading && !heading.hidden,historyTop:heading?.getBoundingClientRect().top,scrollTop:scroller?.scrollTop,scrollerTop:scroller?.getBoundingClientRect().top}; });
   console.log(label,JSON.stringify(data)); return data;
 };
 await page.waitForTimeout(400);
 await state('initial');
 // Exercise native button activation with the real global transport listener installed.
 const toggle = page.locator('button[aria-expanded]');
 const closedBox = await toggle.boundingBox();
 assert(closedBox.height >= 28, 'comfortable click target');
 const disclosure = await toggle.getAttribute('aria-controls');
 assert(disclosure, 'toggle identifies the disclosure it controls');
 assert(await page.locator(`[id="${disclosure}"]`).evaluate(el => el.hidden));
 await toggle.focus();
 await page.keyboard.down('Space');
 await page.keyboard.down('Space');
 assert.equal(await toggle.getAttribute('aria-expanded'), 'false', 'holding Space waits for keyup');
 assert.equal(await toggle.evaluate(el => getComputedStyle(el).translate), '0px', 'pressed button stays still');
 await page.keyboard.up('Space');
 await page.getByRole('button', {name:'Hide history',exact:true}).waitFor();
 assert.equal(await page.evaluate(() => window.pauseCalls || 0), 0, 'Space must not pause playback');
 assert.equal((await toggle.boundingBox()).width, closedBox.width, 'labels keep a stable width');
 assert(!(await page.locator(`[id="${disclosure}"]`).evaluate(el => el.hidden)));
 assert(await toggle.evaluate(el => el === document.activeElement && el.matches(':focus-visible')));
 await page.keyboard.press('Enter');
 await page.getByRole('button', {name:'Show history',exact:true}).waitFor();
 assert.equal(await page.evaluate(() => window.pauseCalls || 0), 0, 'Enter must not pause playback');
 await toggle.evaluate(el => el.blur());
 await page.keyboard.press('Space');
 assert.equal(await page.evaluate(() => window.pauseCalls), 1, 'global Space transport still works');
 let interactionCases = 3;
 for (const dark of [false,true]) {
   await page.evaluate(dark => document.documentElement.classList.toggle('dark',dark),dark);
   await toggle.hover();
   await page.waitForTimeout(180);
   assert.equal(await toggle.evaluate(el => getComputedStyle(el).backgroundColor), 'rgba(0, 0, 0, 0)');
   await toggle.click();
   await page.waitForTimeout(180);
   assert.equal(await toggle.evaluate(el => getComputedStyle(el).backgroundColor), 'rgba(0, 0, 0, 0)');
   await toggle.screenshot({path:resolve(root, `toggle-${dark?'dark':'light'}.png`)});
   await toggle.click();
   interactionCases++;
 }
 await page.evaluate(() => document.documentElement.classList.remove('dark'));
 await page.emulateMedia({reducedMotion:'reduce'});
 assert.equal(await toggle.evaluate(el => getComputedStyle(el).transitionProperty),'none');
 await page.emulateMedia({reducedMotion:'no-preference'});
 interactionCases++;
 await settled();
 // Sample actual geometry on rendered frames, not just the final visibility boolean.
 await page.evaluate(() => window.setHistoryVisible(true));
 await settled();
 const collapse = await toggle.evaluate(async btn => {
   const history = document.getElementById(btn.getAttribute('aria-controls'));
   const scroller = document.getElementById(btn.getAttribute('aria-controls')).parentElement;
   scroller.scrollTop = 0;
   await new Promise(requestAnimationFrame);
   const initial = history.getBoundingClientRect().height;
   const samples = [];
   btn.click();
   const start = performance.now();
   do {
     await new Promise(requestAnimationFrame);
     samples.push({height:history.getBoundingClientRect().height,top:history.nextElementSibling.getBoundingClientRect().top});
   } while (performance.now() - start < 300);
   return {initial,samples,hidden:history.hidden};
 });
 assert(collapse.samples.filter(s => s.height > 1 && s.height < collapse.initial - 1).length >= 2,
   'hiding must pass through intermediate heights instead of disappearing at once');
 assert(collapse.samples.some((s,i) => i && s.top < collapse.samples[i-1].top - 1),
   'the current-track header glides upward as visible history collapses');
 assert(collapse.hidden, 'outgoing rows are removed when the animation ends');
 interactionCases++;
 await page.evaluate(() => window.setHistoryVisible(true));
 await settled();
 const anchored = await toggle.evaluate(async btn => {
   const scroller = document.getElementById(btn.getAttribute('aria-controls')).parentElement;
   const header = document.getElementById(btn.getAttribute('aria-controls')).nextElementSibling;
   scroller.scrollTop += header.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
   const top = header.getBoundingClientRect().top;
   const samples = [];
   btn.click();
   const start = performance.now();
   do {
     await new Promise(requestAnimationFrame);
     samples.push(header.getBoundingClientRect().top);
   } while (performance.now()-start < 300);
   return {top,samples};
 });
 assert(Math.max(...anchored.samples.map(top => Math.abs(top-anchored.top))) <= 1,
   `collapsing offscreen history keeps the playing header anchored throughout: ${JSON.stringify(anchored)}`);
 interactionCases++;
 // Reversal starts at the current height and leaves no pending collapse behind.
 await page.evaluate(() => window.setHistoryVisible(true));
 await page.waitForTimeout(60);
 const reversal = await toggle.evaluate(async btn => {
   const history = document.getElementById(btn.getAttribute('aria-controls'));
   const before = history.getBoundingClientRect().height;
   btn.click();
   await Promise.resolve();
   return {before,after:history.getBoundingClientRect().height,inert:history.inert};
 });
 assert(Math.abs(reversal.before-reversal.after) < 2, 'reversal does not reset the height');
 assert(reversal.inert, 'outgoing history stops accepting focus immediately');
 await page.waitForTimeout(40);
 await page.evaluate(() => window.setHistoryVisible(true));
 await settled();
 assert.equal(await toggle.getAttribute('aria-expanded'),'true');
 assert(await page.locator(`[id="${disclosure}"]`).evaluate(el => !el.hidden && !el.style.height));
 interactionCases++;
 // Reduced motion has no delayed removal or disclosure animation.
 await page.emulateMedia({reducedMotion:'reduce'});
 await page.evaluate(() => window.setHistoryVisible(false));
 await page.waitForTimeout(30);
 assert(await page.locator(`[id="${disclosure}"]`).evaluate(el => el.hidden && !el.style.height));
 assert(await toggle.evaluate(btn => ![...document.getElementById(btn.getAttribute('aria-controls')).parentElement.querySelectorAll('[data-row]')]
   .some(row => row.getAnimations().some(animation => animation.playState === 'running'))),
   'reduced motion also disables row FLIP animations');
 await page.emulateMedia({reducedMotion:'no-preference'});
 interactionCases++;
 // Unmounting mid-collapse must not restore a viewport from the expanded layout.
 await page.evaluate(() => window.setHistoryVisible(true));
 await settled();
 await toggle.evaluate(btn => {document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=400;btn.click();});
 await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
 await page.getByRole('tab',{name:'Queue',exact:true}).click();
 await page.waitForTimeout(250);
 assert.equal(await toggle.getAttribute('aria-expanded'),'false');
 assert((await toggle.evaluate(btn=>document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop)) < 50);
 interactionCases++;
 // A track change during the transition must clear old height/frame work.
 await page.evaluate(() => window.setHistoryVisible(true));
 await page.waitForTimeout(60);
 const interrupted = await toggle.evaluate(async btn => {
   const {playback}=await import('/player.svelte.ts');
   const heading=document.getElementById(btn.getAttribute('aria-controls')).nextElementSibling;
   const before=heading.getBoundingClientRect().top;
   playback.queue.currentIndex=2;
   await new Promise(requestAnimationFrame);
   return {before,after:heading.getBoundingClientRect().top};
 });
 assert(Math.abs(interrupted.after-interrupted.before)<=1,
   `track changes during expansion retain the current heading position: ${JSON.stringify(interrupted)}`);
 await page.waitForTimeout(250);
 assert(await toggle.evaluate(btn => {
   const history=document.getElementById(btn.getAttribute('aria-controls'));
   return !history.hidden && !history.style.height && !history.hasAttribute('data-history-transitioning');
 }));
 await page.evaluate(async () => {
   const {playback}=await import('/player.svelte.ts');
   playback.queue.currentIndex=8;
   window.setHistoryVisible(false);
 });
 await settled();
 interactionCases++;
 await page.getByRole('button',{name:'Show history'}).click();
 await settled();
 await page.getByRole('button',{name:'Hide history'}).waitFor();
 // Inspect history as a user would before leaving the queue.
 await page.getByRole('button',{name:'Hide history'}).evaluate(btn => { document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=0; });
 await page.waitForTimeout(100);
 const before = await state('before switching');
 await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
 await page.getByText('Lyrics fixture').waitFor();
 await page.getByRole('tab',{name:'Queue',exact:true}).click();
 await page.getByRole('button',{name:'Hide history'}).waitFor();
 await page.waitForTimeout(150);
 const after=await state('after switching');
 await page.screenshot({path:resolve(root,'after-switch.png')});
 assert.equal(after.preference, 'true');
 assert.equal(after.expanded, 'true');
 assert.equal(after.scrollTop, before.scrollTop, 'tab switch retains the visible history viewport');
 assert.equal(after.historyTop, before.historyTop);
 let cases = 1;
 const tabs = async () => {
   await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
   await page.getByText('Lyrics fixture').waitFor();
   await page.getByRole('tab',{name:'Queue',exact:true}).click();
   await page.locator('button[aria-expanded]').waitFor();
   await page.waitForTimeout(100);
 };
 for (const shown of [true,false]) {
   if ((await page.locator('button[aria-expanded]').getAttribute('aria-expanded')) !== String(shown))
     await page.locator('button[aria-expanded]').click();
   await page.waitForTimeout(100);
   for (const position of [0,180,650]) {
     await page.locator('button[aria-expanded]').evaluate((btn,top)=>{document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=top;},position);
     await page.waitForTimeout(100);
     const expected = await state(`before ${shown}/${position}`);
     await tabs();
     const actual = await state(`after ${shown}/${position}`);
     assert.equal(actual.preference,String(shown));
     assert.equal(actual.expanded,String(shown));
     assert.equal(actual.historyInDom,shown);
     assert.equal(actual.scrollTop,expected.scrollTop);
     cases++;
   }
 }
 // Immediately above the animation threshold, disclosure must settle without height
 // motion even if hiding history leaves fewer than 200 visible rows.
 await page.evaluate(async()=>{const {playback}=await import('/player.svelte.ts'); playback.queue={items:Array.from({length:201},(_,i)=>({video_id:`boundary-${i}`,title:`Boundary ${i+1}`,artists:'Fixture artist'})),currentIndex:100,playedFrom:0,sourceName:'Boundary queue'};window.setHistoryVisible(false);});
 await page.waitForTimeout(100);
 for (const visible of [true,false]) {
   await toggle.click();
   assert.equal(await toggle.getAttribute('aria-expanded'),String(visible));
   assert(await toggle.evaluate(btn=>{
     const history=document.getElementById(btn.getAttribute('aria-controls'));
     return !history.style.height && !history.hasAttribute('data-history-transitioning');
   }), '201-item disclosure skips the height animation');
 }
 interactionCases++;
 // Use the actual virtualization threshold and a much larger played prefix.
 await page.evaluate(async()=>{const {playback}=await import('/player.svelte.ts'); playback.queue={items:Array.from({length:600},(_,i)=>({video_id:`large-${i}`,title:`Large ${i+1}`,artists:'Fixture artist'})),currentIndex:300,playedFrom:0,sourceName:'Large queue'};});
 await page.locator('button[aria-expanded]').click();
 await page.waitForTimeout(150);
 for (const top of [0,720,14000]) {
   await page.locator('button[aria-expanded]').evaluate((btn,top)=>{document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=top;},top);
   await page.waitForTimeout(100);
   const expected=await state(`large before ${top}`);
   await tabs();
   const actual=await state(`large after ${top}`);
   assert.equal(actual.preference,'true');
   assert.equal(actual.scrollTop,expected.scrollTop);
   cases++;
 }
 // A new queue while lyrics are open must not restore an unrelated old pixel position.
 await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
 await page.evaluate(async()=>{const {playback}=await import('/player.svelte.ts'); playback.queue={items:Array.from({length:30},(_,i)=>({video_id:`replacement-${i}`,title:`Replacement ${i+1}`,artists:'Fixture artist'})),currentIndex:2,playedFrom:0,sourceName:'Replacement'};});
 await page.getByRole('tab',{name:'Queue',exact:true}).click();
 await page.waitForTimeout(150);
 const replacement=await state('replacement');
 assert.equal(replacement.preference,'true');
 assert(replacement.scrollTop<500, 'discard position from replaced queue');
 cases++;
 // In-place index updates and changes from another queue view invalidate the stored viewport.
 for (const change of ['index','visibility']) {
   await page.locator('button[aria-expanded]').evaluate(btn=>{document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=650;});
   await page.waitForTimeout(100);
   await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
   await page.evaluate(async(change)=>{
     if(change==='index') {const {playback}=await import('/player.svelte.ts'); playback.queue.currentIndex=3;}
   },change);
   if(change==='visibility') await page.evaluate(()=>window.setHistoryVisible(false));
   await page.getByRole('tab',{name:'Queue',exact:true}).click();
   await page.waitForTimeout(150);
   const actual=await state(`changed ${change}`);
   assert(actual.scrollTop<500, `discard position after ${change} change`);
   assert.equal(actual.preference,change==='visibility'?'false':'true');
   cases++;
 }

 // With no history rows, an external preference update must not start a phantom transition
 // or discard the viewport when navigating away. Use the real row layout and containment.
 await page.evaluate(async () => {
   const {playback}=await import('/player.svelte.ts');
   playback.queue.currentIndex=0;
   window.setHistoryVisible(false);
 });
 await settled();
 const emptyScroll = await page.evaluate(async () => {
   const {np}=await import('/player.svelte.ts');
   const scroller=[...document.querySelectorAll('h3')]
     .find(h=>h.textContent==='Now Playing').closest('.overflow-y-auto');
   scroller.scrollTop=500;
   const before=scroller.scrollTop;
   window.setHistoryVisible(true);
   await new Promise(requestAnimationFrame);
   const animating=!!document.querySelector('[data-history-transitioning]');
   np.tab='lyrics';
   return {before,animating};
 });
 assert(!emptyScroll.animating, 'empty history does not animate or invalidate scroll memory');
 await page.getByText('Lyrics fixture').waitFor();
 await page.getByRole('tab',{name:'Queue',exact:true}).click();
 await page.waitForTimeout(150);
 assert.equal(await page.evaluate(() => [...document.querySelectorAll('h3')]
   .find(h=>h.textContent==='Now Playing').closest('.overflow-y-auto').scrollTop),emptyScroll.before);
 interactionCases++;
 // Depart in the microtask between the DOM flush and instant anchor correction. A cancelled
 // correction must not restore the old layout's scroll offset with the new visibility.
 for (const [length,reduce] of [[30,true],[600,false]]) {
   await page.emulateMedia({reducedMotion:reduce?'reduce':'no-preference'});
   await page.evaluate(async length => {
     const {playback}=await import('/player.svelte.ts');
     playback.queue={items:Array.from({length},(_,i)=>({video_id:`pending-${i}`,title:`Pending ${i+1}`,artists:'Fixture artist'})),currentIndex:length===600?300:8,playedFrom:0,sourceName:'Pending anchor'};
     window.setHistoryVisible(true);
   },length);
   await page.waitForTimeout(150);
   await toggle.evaluate(btn=>{document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=650;});
   await page.waitForTimeout(50);
   await page.evaluate(async () => {
     const {np}=await import('/player.svelte.ts');
     window.setHistoryVisible(false);
     await Promise.resolve();
     np.tab='lyrics';
   });
   await page.getByText('Lyrics fixture').waitFor();
   await page.getByRole('tab',{name:'Queue',exact:true}).click();
   await page.waitForTimeout(150);
   assert((await toggle.evaluate(btn=>document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop)) < 50,
     'unmount during instant correction must not restore an expanded-layout offset');
   cases++;
 }

 await page.emulateMedia({reducedMotion:'no-preference'});
 await page.evaluate(async () => {
   const {playback}=await import('/player.svelte.ts');
   playback.queue={items:Array.from({length:30},(_,i)=>({video_id:`focus-${i}`,title:`Focus ${i+1}`,artists:'Fixture artist'})),currentIndex:8,playedFrom:0,sourceName:'Focus queue'};
   window.setHistoryVisible(true);
 });
 await settled();
 // A shared preference update can collapse a view with keyboard focus inside its history.
 await toggle.evaluate(btn => {
   document.getElementById(btn.getAttribute('aria-controls')).querySelector('[role="button"]').focus();
   window.setHistoryVisible(false);
 });
 await settled();
 assert(await toggle.evaluate(btn=>document.activeElement===btn),
   'collapsing a focused history returns focus to that view disclosure');
 interactionCases++;
 // Exercise the component catch/toast path, then retry through a later user toggle.
 await page.evaluate(() => {
   const original=Storage.prototype.setItem;
   window.restoreStorage=()=>{Storage.prototype.setItem=original;};
   Storage.prototype.setItem=function(key,value){
     if(key==='appearance') throw new DOMException('Storage unavailable','QuotaExceededError');
     return original.call(this,key,value);
   };
 });
 await toggle.click();
 await settled();
 assert.equal(await toggle.getAttribute('aria-expanded'),'true');
 assert.equal(await page.evaluate(()=>window.historySaveErrors.length),1);
 assert.equal(await page.evaluate(()=>JSON.parse(localStorage.getItem('appearance')).queueHistoryVisible),false);
 await page.evaluate(()=>window.restoreStorage());
 await toggle.click();
 await settled();
 assert.equal(await page.evaluate(()=>JSON.parse(localStorage.getItem('appearance')).queueHistoryVisible),false);
 await toggle.click();
 await settled();
 assert.equal(await page.evaluate(()=>JSON.parse(localStorage.getItem('appearance')).queueHistoryVisible),true);
 interactionCases++;

 // A click on Show history must reveal played tracks without an extra wheel/scroll action.
 // Small histories fit entirely; long histories show recent rows with the current track.
 for (const [length,index,reduce] of [[30,5,false],[80,40,false],[600,300,false],[30,5,true]]) {
   await page.emulateMedia({reducedMotion:reduce?'reduce':'no-preference'});
   await page.evaluate(async ({length,index}) => {
     const {playback}=await import('/player.svelte.ts');
     playback.queue={items:Array.from({length},(_,i)=>({video_id:`reveal-${i}`,title:`Reveal ${i+1}`,artists:'Fixture artist'})),currentIndex:index,playedFrom:0,sourceName:'Reveal queue'};
     window.setHistoryVisible(false);
   },{length,index});
   await settled();
   await toggle.evaluate(btn=>{
     const scroller=document.getElementById(btn.getAttribute('aria-controls')).parentElement;
     scroller.scrollTop+=document.getElementById(btn.getAttribute('aria-controls')).nextElementSibling.getBoundingClientRect().top-scroller.getBoundingClientRect().top;
   });
   await page.waitForTimeout(100);
   await toggle.evaluate(btn=>{
     const history=document.getElementById(btn.getAttribute('aria-controls'));
     window.revealFrames=[];
     const started=performance.now();
     const sample=()=>{
       if(!history.isConnected) return;
       window.revealFrames.push({height:history.getBoundingClientRect().height,top:document.getElementById(btn.getAttribute('aria-controls')).nextElementSibling.getBoundingClientRect().top,animating:history.hasAttribute('data-history-transitioning')});
       if(performance.now()-started<350) requestAnimationFrame(sample);
     };
     requestAnimationFrame(sample);
   });
   await toggle.click();
   await settled();
   await page.waitForTimeout(100);
   if(length<=200 && !reduce) {
     const frames=await page.evaluate(()=>window.revealFrames);
     assert(frames.filter(f=>f.animating && f.height>1).length>=2,'Show animates through rendered intermediate heights');
     const end=frames.findIndex((f,i)=>i && !f.animating && frames[i-1].animating);
     assert(end>0 && Math.abs(frames[end].top-frames[end-1].top)<=2,
       `ending the reveal does not jump after estimated row heights settle: ${JSON.stringify(frames.slice(Math.max(0,end-2),end+2))}`);
   }
   const revealed = await toggle.evaluate(btn=>{
     const scroller=document.getElementById(btn.getAttribute('aria-controls')).parentElement;
     const viewport=scroller.getBoundingClientRect();
     const history=document.getElementById(btn.getAttribute('aria-controls'));
     const bounds=history.getBoundingClientRect();
     const last=history.querySelector('[data-row]:last-child')?.getBoundingClientRect();
     const playing=document.getElementById(btn.getAttribute('aria-controls')).nextElementSibling.nextElementSibling.getBoundingClientRect();
     return {viewport:{top:viewport.top,bottom:viewport.bottom},history:{top:bounds.top,bottom:bounds.bottom},last: last && {top:last.top,bottom:last.bottom},playing:{top:playing.top,bottom:playing.bottom},toggleTop:btn.getBoundingClientRect().top};
   });
   assert(revealed.last && revealed.last.top>=revealed.viewport.top-1 && revealed.last.bottom<=revealed.viewport.bottom+1,
     `Show history exposes recent history without scrolling (${length}/${reduce}): ${JSON.stringify(revealed)}`);
   assert(revealed.playing.bottom<=revealed.viewport.bottom+1 && revealed.toggleTop<revealed.viewport.top && revealed.toggleTop>=0,
     'revealing history keeps the current track and collapse control visible');
   if(index===5) assert(revealed.history.top>=revealed.viewport.top-1,'a five-track history is fully in view');
   if(length===30 && !reduce) await page.screenshot({path:resolve(root,'show-history-without-scrolling.png')});
   await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
   await page.getByRole('tab',{name:'Queue',exact:true}).click();
   await page.waitForTimeout(150);
   const restored=await toggle.evaluate(btn=>document.getElementById(btn.getAttribute('aria-controls')).getBoundingClientRect().bottom);
   assert(Math.abs(restored-revealed.history.bottom)<=1,'tab switch retains the revealed history viewport');
   interactionCases++;
 }
 // The disclosure stays above the scroller at the same coordinates during animation,
 // after scrolling either direction, and at a narrower player width.
 for (const width of [1100,820]) {
   await page.setViewportSize({width,height:800});
   await page.emulateMedia({reducedMotion:'no-preference'});
   await page.evaluate(()=>window.setHistoryVisible(false));
   await settled();
   const fixed = await toggle.boundingBox();
   const stable = await toggle.evaluate(async btn => {
     const history=document.getElementById(btn.getAttribute('aria-controls'));
     const scroller=history.parentElement;
     const before=btn.getBoundingClientRect();
     const samples=[];
     for (const shown of [true,false]) {
       btn.click();
       const start=performance.now();
       do {
         await new Promise(requestAnimationFrame);
         const box=btn.getBoundingClientRect();
         samples.push({x:box.x,y:box.y});
       } while(performance.now()-start<260);
       for (const top of [scroller.scrollHeight,0]) {
         scroller.scrollTop=top;
         await new Promise(requestAnimationFrame);
         const box=btn.getBoundingClientRect();
         samples.push({x:box.x,y:box.y});
       }
     }
     const viewport=scroller.getBoundingClientRect();
     return {samples,bottom:before.bottom,viewportTop:viewport.top};
   });
   assert(stable.bottom<=stable.viewportTop,'disclosure sits above the scrolling rows');
   assert(stable.samples.every(b=>Math.abs(b.x-fixed.x)<=1 && Math.abs(b.y-fixed.y)<=1),
     `disclosure remains fixed through show, hide and scrolling at width ${width}`);
   await toggle.click();
   await settled();
   await page.screenshot({path:resolve(root,`fixed-toolbar-${width}.png`)});
   interactionCases++;
 }
 // The other consumer uses the same fixed disclosure in its 320px overlay panel.
 await page.goto('http://127.0.0.1:5197/?panel');
 await toggle.waitFor();
 await page.waitForTimeout(300);
 const panelBox=await toggle.boundingBox();
 await toggle.evaluate(btn=>{document.getElementById(btn.getAttribute('aria-controls')).parentElement.scrollTop=800;});
 await page.waitForTimeout(100);
 assert.deepEqual(await toggle.boundingBox(),panelBox,'overlay control stays fixed while scrolling');
 await toggle.click();
 await settled();
 assert.deepEqual(await toggle.boundingBox(),panelBox,'overlay control stays fixed when hiding');
 await toggle.click();
 await settled();
 assert.deepEqual(await toggle.boundingBox(),panelBox,'overlay control stays fixed when showing');
 await page.screenshot({path:resolve(root,'fixed-toolbar-panel.png')});
 interactionCases++;
 // One aligned header, without a second History heading in the usual played-prefix case.
 const headerGeometry=await toggle.evaluate(btn=>{
   const history=document.getElementById(btn.getAttribute('aria-controls'));
   const label=document.getElementById(history.getAttribute('aria-labelledby'));
   const now=history.nextElementSibling.querySelector('h3');
   const a=label.getBoundingClientRect(),b=btn.getBoundingClientRect(),c=now.getBoundingClientRect();
   return {left:a.left+(parseFloat(getComputedStyle(label).paddingLeft)||0),nowLeft:c.left,labelMiddle:a.top+a.height/2,buttonMiddle:b.top+b.height/2,
     nestedHeadings:history.querySelectorAll('h3').length,role:history.getAttribute('role')};
 });
 assert.equal(headerGeometry.left,headerGeometry.nowLeft,'History label aligns with queue headings');
 assert(Math.abs(headerGeometry.labelMiddle-headerGeometry.buttonMiddle)<=1,'label and action share a row');
 assert.equal(headerGeometry.nestedHeadings,0,'no duplicate History heading for the normal queue');
 assert.equal(headerGeometry.role,'group','controlled history has an accessible group label');
 interactionCases++;
 // Unplayed earlier rows retain a distinct boundary from actually played history.
 await page.evaluate(async()=>{const {playback}=await import('/player.svelte.ts');playback.queue.playedFrom=3;});
 await settled();
 assert(await toggle.evaluate(btn=>{
   const history=document.getElementById(btn.getAttribute('aria-controls'));
   return history.querySelector('h3')?.textContent.trim()==='History'
     && history.previousElementSibling?.getAttribute('role')==='list'
     && history.previousElementSibling.children.length===3;
 }),'Earlier and History keep separate section boundaries');
 interactionCases++;
 // Exercise a longer installed translation in the real 320px panel.
 await page.evaluate(async path=>{const {setLocale}=await import(path);setLocale('fr');},'/@fs/'+resolve(lib,'i18n.svelte.ts').replaceAll('\\','/'));
 const localized=await toggle.evaluate(btn=>{
   const history=document.getElementById(btn.getAttribute('aria-controls'));
   const label=document.getElementById(history.getAttribute('aria-labelledby'));
   const a=label.getBoundingClientRect(),b=btn.getBoundingClientRect(),panel=btn.closest('aside').getBoundingClientRect();
   return {labelRight:a.right,buttonLeft:b.left,buttonRight:b.right,panelRight:panel.right,height:b.height};
 });
 assert(localized.labelRight<=localized.buttonLeft && localized.buttonRight<=localized.panelRight && localized.height===28,
   'French label and action fit the narrow queue panel without overlap or wrapping');
 await page.waitForTimeout(250); // Let row FLIP from the Earlier fixture change finish before visual QA.
 await page.screenshot({path:resolve(root,'fixed-toolbar-panel-fr.png')});
 interactionCases++;
 assert.deepEqual(pageErrors,[],'no unexpected browser runtime errors');
 writeFileSync(resolve(root,'result.json'),JSON.stringify({before,after,cases,interactionCases},null,2));
 console.log(`History interaction checks: ${interactionCases} scenarios passed (real global shortcuts; mocked transport).`);
 console.log(`Queue/lyrics component checks: ${cases} scenarios passed (real components; mocked playback, track menus, lyrics).`);
} finally { await browser?.close(); await server.close(); }
