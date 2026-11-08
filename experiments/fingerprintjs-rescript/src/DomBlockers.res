open Bindings
open Browser
open Data
open FpDom
open Async

let getFilters = () => {
  let fromB64 = atob
  let filters = emptyObject()
  set(filters, "abpIndo", ["#Iklan-Melayang", "#Kolom-Iklan-728", "#SidebarIklan-wrapper", "[title=\"ALIENBOLA\" i]", fromB64("I0JveC1CYW5uZXItYWRz")])
  set(filters, "abpvn", [".quangcao", "#mobileCatfish", fromB64("LmNsb3NlLWFkcw=="), "[id^=\"bn_bottom_fixed_\"]", "#pmadv"])
  set(filters, "adBlockFinland", [".mainostila", fromB64("LnNwb25zb3JpdA=="), ".ylamainos", fromB64("YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd")])
  set(filters, "adBlockPersian", ["#navbar_notice_50", ".kadr", "TABLE[width=\"140px\"]", "#divAgahi", fromB64("YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd")])
  set(filters, "adBlockWarningRemoval", ["#adblock-honeypot", ".adblocker-root", ".wp_adblock_detect", fromB64("LmhlYWRlci1ibG9ja2VkLWFk"), fromB64("I2FkX2Jsb2NrZXI=")])
  set(filters, "adGuardAnnoyances", [".hs-sosyal", "#cookieconsentdiv", "div[class^=\"app_gdpr\"]", ".as-oil", "[data-cypress=\"soft-push-notification-modal\"]"])
  set(filters, "adGuardBase", [".BetterJsPopOverlay", fromB64("I2FkXzMwMFgyNTA="), fromB64("I2Jhbm5lcmZsb2F0MjI="), fromB64("I2NhbXBhaWduLWJhbm5lcg=="), fromB64("I0FkLUNvbnRlbnQ=")])
  set(filters, "adGuardChinese", [fromB64("LlppX2FkX2FfSA=="), fromB64("YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd"), "#widget-quan", fromB64("YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd"), fromB64("YVtocmVmKj0iLjE5NTZobC5jb20vIl0=")])
  set(filters, "adGuardFrench", ["#pavePub", fromB64("LmFkLWRlc2t0b3AtcmVjdGFuZ2xl"), ".mobile_adhesion", ".widgetadv", fromB64("LmFkc19iYW4=")])
  set(filters, "adGuardGerman", ["aside[data-portal-id=\"leaderboard\"]"])
  set(filters, "adGuardJapanese", ["#kauli_yad_1", fromB64("YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0="), fromB64("Ll9wb3BJbl9pbmZpbml0ZV9hZA=="), fromB64("LmFkZ29vZ2xl"), fromB64("Ll9faXNib29zdFJldHVybkFk")])
  set(filters, "adGuardMobile", [fromB64("YW1wLWF1dG8tYWRz"), fromB64("LmFtcF9hZA=="), "amp-embed[type=\"24smi\"]", "#mgid_iframe1", fromB64("I2FkX2ludmlld19hcmVh")])
  set(filters, "adGuardRussian", [fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0="), fromB64("LnJlY2xhbWE="), "div[id^=\"smi2adblock\"]", fromB64("ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd"), "#psyduckpockeball"])
  set(filters, "adGuardSocial", [fromB64("YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0="), fromB64("YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0="), ".etsy-tweet", "#inlineShare", ".popup-social"])
  set(filters, "adGuardSpanishPortuguese", ["#barraPublicidade", "#Publicidade", "#publiEspecial", "#queTooltip", ".cnt-publi"])
  set(filters, "adGuardTrackingProtection", ["#qoo-counter", fromB64("YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=="), fromB64("YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0="), fromB64("YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=="), "#top100counter"])
  set(filters, "adGuardTurkish", ["#backkapat", fromB64("I3Jla2xhbWk="), fromB64("YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0="), fromB64("YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd"), fromB64("YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ==")])
  set(filters, "bulgarian", [fromB64("dGQjZnJlZW5ldF90YWJsZV9hZHM="), "#ea_intext_div", ".lapni-pop-over", "#xenium_hot_offers"])
  set(filters, "easyList", [".yb-floorad", fromB64("LndpZGdldF9wb19hZHNfd2lkZ2V0"), fromB64("LnRyYWZmaWNqdW5reS1hZA=="), ".textad_headline", fromB64("LnNwb25zb3JlZC10ZXh0LWxpbmtz")])
  set(filters, "easyListChina", [fromB64("LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=="), fromB64("LmZyb250cGFnZUFkdk0="), "#taotaole", "#aafoot.top_box", ".cfa_popup"])
  set(filters, "easyListCookie", [".ezmob-footer", ".cc-CookieWarning", "[data-cookie-number]", fromB64("LmF3LWNvb2tpZS1iYW5uZXI="), ".sygnal24-gdpr-modal-wrap"])
  set(filters, "easyListCzechSlovak", ["#onlajny-stickers", fromB64("I3Jla2xhbW5pLWJveA=="), fromB64("LnJla2xhbWEtbWVnYWJvYXJk"), ".sklik", fromB64("W2lkXj0ic2tsaWtSZWtsYW1hIl0=")])
  set(filters, "easyListDutch", [fromB64("I2FkdmVydGVudGll"), fromB64("I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=="), ".adstekst", fromB64("YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0="), "#semilo-lrectangle"])
  set(filters, "easyListGermany", ["#SSpotIMPopSlider", fromB64("LnNwb25zb3JsaW5rZ3J1ZW4="), fromB64("I3dlcmJ1bmdza3k="), fromB64("I3Jla2xhbWUtcmVjaHRzLW1pdHRl"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0=")])
  set(filters, "easyListItaly", [fromB64("LmJveF9hZHZfYW5udW5jaQ=="), ".sb-box-pubbliredazionale", fromB64("YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ==")])
  set(filters, "easyListLithuania", [fromB64("LnJla2xhbW9zX3RhcnBhcw=="), fromB64("LnJla2xhbW9zX251b3JvZG9z"), fromB64("aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd"), fromB64("aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd"), fromB64("aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd")])
  set(filters, "estonian", [fromB64("QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ==")])
  set(filters, "fanboyAnnoyances", ["#ac-lre-player", ".navigate-to-top", "#subscribe_popup", ".newsletter_holder", "#back-top"])
  set(filters, "fanboyAntiFacebook", [".util-bar-module-firefly-visible"])
  set(filters, "fanboyEnhancedTrackers", [".open.pushModal", "#issuem-leaky-paywall-articles-zero-remaining-nag", "#sovrn_container", "div[class$=\"-hide\"][zoompage-fontsize][style=\"display: block;\"]", ".BlockNag__Card"])
  set(filters, "fanboySocial", ["#FollowUs", "#meteored_share", "#social_follow", ".article-sharer", ".community__social-desc"])
  set(filters, "frellwitSwedish", [fromB64("YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=="), fromB64("YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=="), "article.category-samarbete", fromB64("ZGl2LmhvbGlkQWRz"), "ul.adsmodern"])
  set(filters, "greekAdBlock", [fromB64("QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd"), fromB64("QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=="), fromB64("QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd"), "DIV.agores300", "TABLE.advright"])
  set(filters, "hungarian", ["#cemp_doboz", ".optimonk-iframe-container", fromB64("LmFkX19tYWlu"), fromB64("W2NsYXNzKj0iR29vZ2xlQWRzIl0="), "#hirdetesek_box"])
  set(filters, "iDontCareAboutCookies", [".alert-info[data-block-track*=\"CookieNotice\"]", ".ModuleTemplateCookieIndicator", ".o--cookies--container", "#cookies-policy-sticky", "#stickyCookieBar"])
  set(filters, "icelandicAbp", [fromB64("QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ==")])
  set(filters, "latvian", [
    fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0OiA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0="),
    fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6IDMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ=="),
  ])
  set(filters, "listKr", [fromB64("YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0="), fromB64("I2xpdmVyZUFkV3JhcHBlcg=="), fromB64("YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=="), fromB64("aW5zLmZhc3R2aWV3LWFk"), ".revenue_unit_item.dable"])
  set(filters, "listeAr", [fromB64("LmdlbWluaUxCMUFk"), ".right-and-left-sponsers", fromB64("YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=="), fromB64("YVtocmVmKj0iYm9vcmFxLm9yZyJd"), fromB64("YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd")])
  set(filters, "listeFr", [fromB64("YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=="), fromB64("I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=="), fromB64("YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0="), ".site-pub-interstitiel", "div[id^=\"crt-\"][data-criteo-id]"])
  set(filters, "officialPolish", ["#ceneo-placeholder-ceneo-12", fromB64("W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd"), fromB64("YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=="), fromB64("YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=="), fromB64("ZGl2I3NrYXBpZWNfYWQ=")])
  set(filters, "ro", [fromB64("YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0="), fromB64("YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd"), "a[href^=\"/url/\"]"])
  set(filters, "ruAd", [fromB64("YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd"), fromB64("YVtocmVmKj0iLy91dGltZy5ydS8iXQ=="), fromB64("YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0="), "#pgeldiz", ".yandex-rtb-block"])
  set(filters, "thaiAds", ["a[href*=macau-uta-popup]", fromB64("I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=="), fromB64("LmFkczMwMHM="), ".bumq", ".img-kosana"])
  set(filters, "webAnnoyancesUltralist", ["#mod-social-share-2", "#social-tools", fromB64("LmN0cGwtZnVsbGJhbm5lcg=="), ".zergnet-recommend", ".yt.btn-link.btn-md.btn"])
  filters
}

let isApplicable = () => isWebKit() || isAndroid()

let forceShow = element => {
  setProperty(get(element, "style"), "visibility", "hidden", "important")
  setProperty(get(element, "style"), "display", "block", "important")
}

let getBlockedSelectors = async (selectors: array<string>) => {
  let d = document()
  let root = createElement(d, "div")
  let elements = newArray(arrayLength(selectors))
  let blockedSelectors = emptyObject()
  forceShow(root)
  let rec create = i =>
    if i >= arrayLength(selectors) {
      ()
    } else {
      let element = selectorToElement(getInt(selectors, i))
      if asString(get(element, "tagName")) == "DIALOG" {
        call0(element, "show")->ignore
      }
      let holder = createElement(d, "div")
      forceShow(holder)
      appendChild(holder, element)->ignore
      appendChild(root, holder)->ignore
      jsSetIndex(elements, i, element)
      create(i + 1)
    }
  create(0)
  let rec waitForBody = async () =>
    if isFalsy(get(d, "body")) {
      let _ = await wait(50, jsUndefined)
      await waitForBody()
    } else {
      ()
    }
  await waitForBody()
  appendChild(get(d, "body"), root)->ignore
  let rec check = i =>
    if i >= arrayLength(selectors) {
      ()
    } else {
      if isFalsy(get(getInt(asArray(elements), i), "offsetParent")) {
        set(blockedSelectors, getInt(selectors, i), true)
      }
      check(i + 1)
    }
  let outcome = runAction(() => check(0))
  let parent = get(root, "parentNode")
  if isTruthy(parent) {
    removeChild(parent, root)->ignore
  }
  if !asBool(getInt(outcome, 0)) {
    throwJs(getInt(outcome, 1))
  }
  blockedSelectors
}

let printDebug = (filters, blockedSelectors) => {
  let message = ref("DOM blockers debug:\n```")
  let filterNames = objectKeys(filters)
  let rec loop = i =>
    if i >= arrayLength(filterNames) {
      ()
    } else {
      let filterName = getInt(filterNames, i)
      message := message.contents ++ "\n" ++ filterName ++ ":"
      let selectors = asArray(get(filters, filterName))
      let rec selLoop = j =>
        if j >= arrayLength(selectors) {
          ()
        } else {
          let selector = getInt(selectors, j)
          message :=
            message.contents ++
            "\n  " ++ if isTruthy(get(blockedSelectors, selector)) {
              "🚫"
            } else {
              "➡️"
            } ++ " " ++ selector
          selLoop(j + 1)
        }
      selLoop(0)
      loop(i + 1)
    }
  loop(0)
  call1(get(globalThis, "console"), "log", message.contents ++ "\n```")->ignore
}

let getDomBlockers = async (): any => {
  let options: any = %raw(`arguments[0]`)
  if !isApplicable() {
    jsUndefined
  } else {
    let filters = getFilters()
    let filterNames = objectKeys(filters)
    let allSelectors = []
    let rec collect = i =>
      if i >= arrayLength(filterNames) {
        ()
      } else {
        let part = asArray(get(filters, getInt(filterNames, i)))
        let rec copy = j =>
          if j >= arrayLength(part) {
            ()
          } else {
            arrayPush(allSelectors, getInt(part, j))->ignore
            copy(j + 1)
          }
        copy(0)
        collect(i + 1)
      }
    collect(0)
    let blockedSelectors = await getBlockedSelectors(allSelectors)
    if isTruthy(options) && isTruthy(get(options, "debug")) {
      printDebug(filters, blockedSelectors)
    }
    let activeBlockers = arrayFilter(filterNames, (filterName, _) => {
      let selectors = asArray(get(filters, filterName))
      let blockedCount = countTruthy(arrayMap(selectors, (selector, _) => get(blockedSelectors, selector)))
      asFloat(blockedCount) > asFloat(arrayLength(selectors)) *. 0.6
    })
    identity(arraySort(activeBlockers))
  }
}
