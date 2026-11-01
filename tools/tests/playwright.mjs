// Loads Playwright for the browser tests. PLAYWRIGHT_MODULE (a package directory such as
// /path/to/node_modules/playwright, or a module name) wins; otherwise Node's normal
// resolution from the working directory and from this repository (node_modules, NODE_PATH).
// chromiumOptions() picks the Chromium the tests launch; launchWasm() a browser that can compile
// WebAssembly.
import {existsSync} from 'node:fs';
import {createRequire} from 'node:module';
import {join, resolve} from 'node:path';

const HELP='Install it with `npm install playwright` (in the repository or a parent folder; '
  +'a global install needs NODE_PATH set to the global node_modules) or set PLAYWRIGHT_MODULE '
  +'to the playwright package directory.';

export function loadPlaywright() {
  const fromCwd=createRequire(resolve(process.cwd(),'package.json'));
  const explicit=process.env.PLAYWRIGHT_MODULE;
  if(explicit) {
    try {return fromCwd(explicit);}
    catch(error) {throw new Error(`PLAYWRIGHT_MODULE=${explicit} could not be loaded: ${error.message}`);}
  }
  for(const load of [fromCwd,createRequire(import.meta.url)]) {
    try {return load('playwright');}
    catch(error) {if(!error.message.startsWith("Cannot find module 'playwright'"))throw error;}
  }
  throw new Error(`Playwright not found. ${HELP}`);
}

// Where Playwright's channel 'chrome' looks for Google Chrome (on Windows below each prefix).
const CHROME={
  linux:'/opt/google/chrome/chrome',
  darwin:'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  win32:'\\Google\\Chrome\\Application\\chrome.exe',
};

function chromeInstalled() {
  const path=CHROME[process.platform];
  if(!path)return false;
  const env=process.env;
  const prefixes=process.platform==='win32'
    ? [env.LOCALAPPDATA,env.PROGRAMFILES,env['PROGRAMFILES(X86)'],
       env.HOMEDRIVE&&env.HOMEDRIVE+'\\Program Files',env.HOMEDRIVE&&env.HOMEDRIVE+'\\Program Files (x86)'].filter(Boolean)
    : [''];
  return prefixes.some(prefix=>existsSync(join(prefix,path)));
}

// Launch options for Chromium: CHROME_PATH (an executable) if set, otherwise an installed
// Google Chrome (channel 'chrome'), otherwise the Chromium Playwright installed
// (`npx playwright install chromium`, below PLAYWRIGHT_BROWSERS_PATH if that is set).
//   pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})})
export function chromiumOptions() {
  if(process.env.CHROME_PATH)return {executablePath:process.env.CHROME_PATH};
  return chromeInstalled() ? {channel:'chrome'} : {};
}

// type.launch(options), a browser that can compile WebAssembly: Playwright's Firefox starts now
// and then without a compiler (WebAssembly.compile throws "no WebAssembly compiler available" for
// the whole run of that browser, seen in up to a quarter of the starts; a reload does not help, a
// new start does).
export async function launchWasm(type,options) {
  const module='new Uint8Array([0,97,115,109,1,0,0,0])';
  for(let tries=1;;tries++) {
    const browser=await type.launch(options);
    const page=await browser.newPage();
    await page.goto('data:text/html,<!doctype html><title>x</title>');
    const error=await page.evaluate(`(async()=>{try{await WebAssembly.compile(${module});return '';}catch(e){return String(e.message);}})()`);
    // (Firefox's session store may trip over a window closed this soon: harmless)
    await page.close().catch(()=>{});
    if(!error) return browser;
    await browser.close();
    if(tries>=8) throw new Error(`${type.name()}: ${error} (${tries} starts)`);
  }
}
