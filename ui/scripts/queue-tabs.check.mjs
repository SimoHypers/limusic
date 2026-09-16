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
 page.on('pageerror',e=>console.error('PAGEERROR',e));
 await page.goto('http://127.0.0.1:5197');
 await page.getByRole('button',{name:'Show history'}).waitFor();
 const settled = () => page.waitForFunction(() => !document.querySelector('[data-history-transitioning]'));
 const state = async (label) => {
   await settled();
   const data = await page.evaluate(() => { const btn=document.querySelector('button[aria-expanded]'); const heading=[...document.querySelectorAll('h3')].find(e=>e.textContent==='History'); const scroller=btn?.closest('.overflow-y-auto'); return {preference:document.querySelector('#preference').getAttribute('data-visible'),button:btn?.textContent.trim(),expanded:btn?.getAttribute('aria-expanded'),historyInDom:!!heading,historyTop:heading?.getBoundingClientRect().top,scrollTop:scroller?.scrollTop,scrollerTop:scroller?.getBoundingClientRect().top}; });
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
   const scroller = btn.closest('.overflow-y-auto');
   scroller.scrollTop = 0;
   await new Promise(requestAnimationFrame);
   const initial = history.getBoundingClientRect().height;
   const samples = [];
   btn.click();
   const start = performance.now();
   do {
     await new Promise(requestAnimationFrame);
     samples.push({height:history.getBoundingClientRect().height,top:btn.getBoundingClientRect().top});
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
   const scroller = btn.closest('.overflow-y-auto');
   const header = btn.parentElement;
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
 assert(await toggle.evaluate(btn => ![...btn.closest('.overflow-y-auto').querySelectorAll('[data-row]')]
   .some(row => row.getAnimations().some(animation => animation.playState === 'running'))),
   'reduced motion also disables row FLIP animations');
 await page.emulateMedia({reducedMotion:'no-preference'});
 interactionCases++;
 // Unmounting mid-collapse must not restore a viewport from the expanded layout.
 await page.evaluate(() => window.setHistoryVisible(true));
 await settled();
 await toggle.evaluate(btn => {btn.closest('.overflow-y-auto').scrollTop=400;btn.click();});
 await page.getByRole('tab',{name:'Lyrics',exact:true}).click();
 await page.getByRole('tab',{name:'Queue',exact:true}).click();
 await page.waitForTimeout(250);
 assert.equal(await toggle.getAttribute('aria-expanded'),'false');
 assert((await toggle.evaluate(btn=>btn.closest('.overflow-y-auto').scrollTop)) < 50);
 interactionCases++;
 // A track change during the transition must clear old height/frame work.
 await page.evaluate(() => window.setHistoryVisible(true));
 await page.waitForTimeout(60);
 const interrupted = await toggle.evaluate(async btn => {
   const {playback}=await import('/player.svelte.ts');
   const heading=btn.parentElement;
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
 await page.getByRole('button',{name:'Hide history'}).evaluate(btn => { btn.closest('.overflow-y-auto').scrollTop=0; });
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
     await page.locator('button[aria-expanded]').evaluate((btn,top)=>{btn.closest('.overflow-y-auto').scrollTop=top;},position);
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
   await page.locator('button[aria-expanded]').evaluate((btn,top)=>{btn.closest('.overflow-y-auto').scrollTop=top;},top);
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
   await page.locator('button[aria-expanded]').evaluate(btn=>{btn.closest('.overflow-y-auto').scrollTop=650;});
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
   await toggle.evaluate(btn=>{btn.closest('.overflow-y-auto').scrollTop=650;});
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
   assert((await toggle.evaluate(btn=>btn.closest('.overflow-y-auto').scrollTop)) < 50,
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
 writeFileSync(resolve(root,'result.json'),JSON.stringify({before,after,cases,interactionCases},null,2));
 console.log(`History interaction checks: ${interactionCases} scenarios passed (real global shortcuts; mocked transport).`);
 console.log(`Queue/lyrics component checks: ${cases} scenarios passed (real components; mocked playback, track menus, lyrics).`);
} finally { await browser?.close(); await server.close(); }
