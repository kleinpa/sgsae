// End-to-end check that the built page really runs the console in a browser:
// the wasm module loads, the home cartridge boots, the PPU's output reaches the
// canvas, the keyboard reaches the guest, and following a link swaps in another
// cartridge.
//
// Serves argv[2] (default ./dist) over HTTP and drives headless Chrome.
// CHROME_BIN and CHROMEDRIVER_BIN override the browser and driver binaries;
// without them selenium-webdriver locates them itself.

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { Builder, By, Key, until } from "selenium-webdriver";
import chrome from "selenium-webdriver/chrome.js";
import { Level, Preferences, Type } from "selenium-webdriver/lib/logging.js";

const ROOT = process.argv[2] ?? "dist";
const TIMEOUT = 30000;

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".wasm": "application/wasm",
  ".css": "text/css",
  ".json": "application/json",
};

const missing = [];
/** Paths the server handed over, so a test can assert nothing was refetched. */
const served = [];

/** Serve `ROOT` on an ephemeral port. Correct MIME types matter: the browser
 *  refuses to instantiate a wasm module served as anything but application/wasm. */
function serve() {
  const server = createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
    const file = join(ROOT, path === "/" ? "index.html" : path);
    try {
      const body = await readFile(file);
      served.push(path);
      res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
      res.end(body);
    } catch {
      // An address like /system/seq is a page, not a file: serve the app and
      // let it read the path. Anything with an extension really is missing.
      if (!extname(path) && path !== "/favicon.ico") {
        const app = await readFile(join(ROOT, "index.html"));
        res.writeHead(200, { "content-type": "text/html" });
        res.end(app);
        return;
      }
      if (path !== "/favicon.ico") missing.push(path);
      res.writeHead(404).end("not found");
    }
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve(server)));
}

const failures = [];
const fail = (message) => failures.push(message);

/** Console messages the browser itself considers errors, minus the favicon. */
async function consoleErrors(driver) {
  const entries = await driver.manage().logs().get("browser");
  return entries
    .filter((e) => e.level.name === "SEVERE" && !e.message.includes("/favicon.ico"))
    .map((e) => e.message);
}

/** Summarise what is currently on the console's screen.
 *
 *  The screen is a WebGL canvas, so its pixels come back by drawing it into a
 *  scratch 2D canvas rather than by asking it for a 2D context. */
const SURVEY = `
  const canvas = document.querySelector("canvas#screen");
  if (!canvas) return null;
  const scratch = document.createElement("canvas");
  scratch.width = canvas.width;
  scratch.height = canvas.height;
  const ctx = scratch.getContext("2d");
  ctx.drawImage(canvas, 0, 0);
  const data = ctx.getImageData(0, 0, scratch.width, scratch.height).data;
  const colours = new Set();
  let opaque = 0, flame = 0, checksum = 0;
  for (let i = 0; i < data.length; i += 4) {
    const rgb = (data[i] << 16) | (data[i + 1] << 8) | data[i + 2];
    colours.add(rgb);
    if (data[i + 3] === 255) opaque += 1;
    // The exhaust plume has a palette entry to itself, so that a lit engine is
    // distinguishable from a red instrument reading: rgb(31, 20, 2) widened.
    if (data[i] === 255 && data[i + 1] === 165 && data[i + 2] === 16) flame += 1;
    checksum = (checksum + rgb * (i + 1)) | 0;
  }
  return { width: canvas.width, height: canvas.height, renderer: canvas.dataset.renderer,
           colours: colours.size, opaque, flame, checksum, pixels: data.length / 4 };
`;

const survey = (driver) => driver.executeScript(SURVEY);
const settle = (ms) => new Promise((done) => setTimeout(done, ms));

const server = await serve();
const port = server.address().port;

const options = new chrome.Options()
  .addArguments("--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--disable-gpu")
  .setLoggingPrefs(new Preferences().setLevel(Type.BROWSER, Level.ALL));
if (process.env.CHROME_BIN) options.setChromeBinaryPath(process.env.CHROME_BIN);

const builder = new Builder().forBrowser("chrome").setChromeOptions(options);
if (process.env.CHROMEDRIVER_BIN) {
  builder.setChromeService(new chrome.ServiceBuilder(process.env.CHROMEDRIVER_BIN));
}
const driver = await builder.build();

try {
  await driver.get(`http://127.0.0.1:${port}/system/lander`);

  // Booting is asynchronous: the wasm module and the cartridge both have to be
  // fetched before anything is drawn.
  let screen = null;
  try {
    await driver.wait(until.elementLocated(By.css("canvas#screen")), TIMEOUT);
    await driver.wait(async () => {
      screen = await survey(driver);
      return screen && screen.colours >= 4;
    }, TIMEOUT);
  } catch (error) {
    fail(`the game never drew a scene (${error.name})`);
  }

  if (screen) {
    // The GPU magnifies, so the drawing buffer is the display size — a whole
    // multiple of the console's 240x160, whatever the window happens to be.
    if (screen.width % 240 !== 0 || screen.height % 160 !== 0) {
      fail(`canvas is ${screen.width}x${screen.height}, not a multiple of 240x160`);
    }
    if (!String(screen.renderer).startsWith("webgl2")) {
      fail(`renderer fell back to ${screen.renderer}; the GPU path was not exercised`);
    }
    if (screen.opaque !== screen.pixels) {
      fail(`${screen.pixels - screen.opaque} pixels were left transparent`);
    }
    if (screen.flame !== 0) {
      fail("the exhaust was lit before anything was pressed");
    }
    // The default shader is a straight nearest-neighbour passthrough, so the
    // screen holds exactly the palette entries the PPU drew. Any filtering or
    // blending on the GPU path would multiply this into the hundreds.
    if (screen.colours > 16) {
      fail(`display showed ${screen.colours} colours; the upscale is not exact`);
    }

    // The lander falls under gravity, so the picture has to keep changing.
    await settle(500);
    const later = await survey(driver);
    if (later.checksum === screen.checksum) {
      fail("the picture never changed; the game is not running");
    }

    // The page is the screen and nothing else: no help text, no chrome, and
    // the picture centred on a dark ground.
    const chrome_ = await driver.executeScript(`
      const canvas = document.querySelector("canvas#screen");
      const rect = canvas.getBoundingClientRect();
      return {
        paragraphs: document.querySelectorAll("p").length,
        background: getComputedStyle(document.body).backgroundColor,
        left: Math.round(rect.left),
        right: Math.round(document.documentElement.clientWidth - rect.right),
        top: Math.round(rect.top),
        bottom: Math.round(document.documentElement.clientHeight - rect.bottom),
        scale: canvas.width / 240,
      };
    `);
    if (chrome_.paragraphs !== 0) {
      fail(`the page still has ${chrome_.paragraphs} lines of text on it`);
    }
    if (chrome_.background !== "rgb(42, 42, 45)") {
      fail(`background is ${chrome_.background}, want dark grey`);
    }
    if (Math.abs(chrome_.left - chrome_.right) > 1 || Math.abs(chrome_.top - chrome_.bottom) > 1) {
      fail(`the screen is not centred: ${JSON.stringify(chrome_)}`);
    }
    // A whole-number magnification, so the picture stays on the pixel grid.
    if (chrome_.scale !== Math.floor(chrome_.scale) || chrome_.scale < 1) {
      fail(`magnification is ${chrome_.scale}, want a whole number`);
    }

    // Audio must start from a keystroke, not only a click. These cartridges
    // are played with the keyboard, so a player who never reaches for the
    // mouse would otherwise never hear anything. Checked here, before the
    // test makes any gesture of its own.
    const audioState = () => driver.executeScript("return document.body.dataset.audio");
    if ((await audioState()) !== "suspended") {
      fail(`audio should start suspended, was ${await audioState()}`);
    }
    await driver.actions().sendKeys(Key.ARROW_LEFT).perform();
    await settle(600);
    if ((await audioState()) !== "running") {
      fail(`a keystroke did not start audio; state is ${await audioState()}`);
    }

    // Alt+Enter is the host's, not the guest's: it must not also count as the
    // Enter that opens a program.
    await driver.actions().keyDown(Key.ALT).sendKeys(Key.ENTER).keyUp(Key.ALT).perform();
    await settle(500);
    if ((await driver.executeScript("return document.body.dataset.cart")) !== "/system/lander") {
      fail("Alt+Enter reached the guest and changed what was running");
    }

    // Holding thrust must reach the guest, which lights the exhaust.
    //
    // Everything above costs a second or so of wall clock, and the lander is
    // falling the whole time — left alone it reaches the ground in under four
    // seconds, and a wreck draws no exhaust however hard thrust is held. Enter
    // starts a fresh round, so the engine is tested against a craft that is
    // still flying rather than against whatever gravity left behind. The game
    // ignores the restart for the first forty frames of a finished round, so
    // that the result can be read, hence the wait before pressing.
    await driver.findElement(By.css("body")).click();
    await settle(800);
    await driver.actions().sendKeys(Key.ENTER).perform();
    await settle(300);

    await driver.actions().keyDown(Key.ARROW_UP).perform();
    await settle(300);
    const burning = await survey(driver);
    await driver.actions().keyUp(Key.ARROW_UP).perform();

    if (burning.flame === 0) {
      fail("holding ArrowUp did not light the exhaust; input is not reaching the guest");
    }

    await settle(300);
    const coasting = await survey(driver);
    if (coasting.flame !== 0) {
      fail("the exhaust stayed lit after the key was released");
    }

    // A different key on a different byte of the keyboard bitmap must reach
    // the guest too — ArrowUp alone would pass even if only one code worked.
    // A fresh round again, for the same reason as above.
    await settle(800);
    await driver.actions().sendKeys(Key.ENTER).perform();
    await settle(300);
    await driver.actions().keyDown("w").perform();
    await settle(300);
    const wasd = await survey(driver);
    await driver.actions().keyUp("w").perform();
    if (wasd.flame === 0) {
      fail("W did not thrust; only part of the keyboard is reaching the guest");
    }
  }

  // Navigation: boot the supervisor, open a program, and check the console
  // really swapped programs rather than just redrawing.
  await driver.get(`http://127.0.0.1:${port}/`);
  try {
    await driver.wait(async () => {
      const home = await survey(driver);
      return home && home.colours >= 4;
    }, TIMEOUT);
    const home = await survey(driver);

    await driver.findElement(By.css("body")).click();
    await driver.actions().sendKeys(Key.ENTER).perform();

    // The lander's sky is near-black and its terrain grey; the home page is
    // blue-tinted. Waiting for the picture to change is enough to know the
    // swap happened, and the URL confirms which way it went.
    await driver.wait(async () => {
      const now = await survey(driver);
      return now && now.checksum !== home.checksum;
    }, TIMEOUT);

    const url = await driver.getCurrentUrl();
    if (!url.endsWith("/system/lander")) {
      fail(`opening a program did not record the address: ${url}`);
    }

    // Quitting must hand the machine back to the supervisor — and must not
    // fetch anything, because the supervisor never left memory.
    const before = missing.length;
    served.length = 0;
    await driver.actions().sendKeys(Key.ESCAPE).perform();
    await settle(1200);
    const cart = await driver.executeScript("return document.body.dataset.cart");
    if (cart !== "") {
      fail(`quitting did not return to the supervisor; address is ${cart}`);
    }
    if (served.some((path) => path.endsWith(".elf"))) {
      fail(`returning to the supervisor refetched ${served.join(", ")}`);
    }

    // The back button leaves a program too, without the program agreeing.
    await driver.actions().sendKeys(Key.ENTER).perform();
    await settle(1400);
    if ((await driver.executeScript("return document.body.dataset.cart")) === "") {
      fail("nothing opened, so the back button has nothing to leave");
    }
    await driver.navigate().back();
    await settle(1400);
    const afterBack = await driver.executeScript("return document.body.dataset.cart");
    if (afterBack !== "") {
      fail(`the back button did not return to the supervisor; still on ${afterBack}`);
    }
    if (missing.length !== before) {
      fail("returning to the supervisor asked for something that is not there");
    }

    // Holding the key that quits a program must not then trip whatever the
    // supervisor has under that key. A person holds a key for several frames.
    await driver.get(`http://127.0.0.1:${port}/system/io`);
    await driver.wait(async () => {
      const booted = await survey(driver);
      return booted && booted.colours >= 4;
    }, TIMEOUT);
    await driver.findElement(By.css("body")).click();

    await driver.actions().keyDown(Key.ESCAPE).perform();
    await settle(1500);
    await driver.actions().keyUp(Key.ESCAPE).perform();
    await settle(400);

    const landed = await driver.executeScript("return document.body.dataset.cart");
    if (landed !== "") {
      fail(`holding the exit key ran past the supervisor to ${landed}`);
    }
    if (missing.length) {
      fail(`navigation requested files the build did not produce: ${missing.join(", ")}`);
    }
  } catch (error) {
    fail(`following a link from the home page failed (${error.name})`);
  }

  if (missing.length) {
    fail(`page requested files the build did not produce: ${missing.join(", ")}`);
  }
  for (const message of await consoleErrors(driver)) {
    fail(`browser console: ${message}`);
  }
} finally {
  await driver.quit();
  server.close();
}

if (failures.length) {
  console.error(`\nFAILED (${failures.length}):`);
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log("--- the game runs in the browser ---");
