import { createRequire } from 'node:module'
import { resolve } from 'node:path'

export function loadWebDriver(oracleRoot) {
  const require = createRequire(resolve(oracleRoot, 'package.json'))
  return {
    webdriver: require('selenium-webdriver'),
    chrome: require('selenium-webdriver/chrome'),
    firefox: require('selenium-webdriver/firefox'),
  }
}

export async function createDriver(browser, oracleRoot, { headed = false } = {}) {
  const { webdriver, chrome, firefox } = loadWebDriver(oracleRoot)
  const builder = new webdriver.Builder().forBrowser(browser)
  if (browser === 'chrome') {
    const options = new chrome.Options()
    if (!headed) options.addArguments('--headless=new')
    options.addArguments('--disable-background-timer-throttling', '--disable-renderer-backgrounding')
    builder.setChromeOptions(options)
  } else if (browser === 'firefox') {
    const options = new firefox.Options()
    if (!headed) options.addArguments('-headless')
    builder.setFirefoxOptions(options)
  } else if (browser !== 'safari') {
    throw new Error(`Unsupported browser: ${browser}`)
  }
  const driver = await builder.build()
  await driver.manage().setTimeouts({ pageLoad: 60000, script: 120000 })
  return driver
}

export async function browserIdentity(driver) {
  const capabilities = await driver.getCapabilities()
  return {
    name: capabilities.get('browserName'),
    version: capabilities.get('browserVersion'),
    platform: capabilities.get('platformName'),
  }
}
