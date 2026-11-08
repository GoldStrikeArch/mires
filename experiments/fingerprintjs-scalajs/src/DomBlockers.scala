package fp

import scala.scalajs.js

object DomBlockers {
  def getFilters(): js.Dictionary[js.Array[String]] = {
    def fromB64(s: String): String = Host.atob(s)
    js.Dictionary(
      "abpIndo" -> js.Array(
        "#Iklan-Melayang",
        "#Kolom-Iklan-728",
        "#SidebarIklan-wrapper",
        """[title="ALIENBOLA" i]""",
        fromB64("I0JveC1CYW5uZXItYWRz"),
      ),
      "abpvn" -> js.Array(".quangcao", "#mobileCatfish", fromB64("LmNsb3NlLWFkcw=="), """[id^="bn_bottom_fixed_"]""", "#pmadv"),
      "adBlockFinland" -> js.Array(
        ".mainostila",
        fromB64("LnNwb25zb3JpdA=="),
        ".ylamainos",
        fromB64("YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd"),
      ),
      "adBlockPersian" -> js.Array(
        "#navbar_notice_50",
        ".kadr",
        """TABLE[width="140px"]""",
        "#divAgahi",
        fromB64("YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd"),
      ),
      "adBlockWarningRemoval" -> js.Array(
        "#adblock-honeypot",
        ".adblocker-root",
        ".wp_adblock_detect",
        fromB64("LmhlYWRlci1ibG9ja2VkLWFk"),
        fromB64("I2FkX2Jsb2NrZXI="),
      ),
      "adGuardAnnoyances" -> js.Array(
        ".hs-sosyal",
        "#cookieconsentdiv",
        """div[class^="app_gdpr"]""",
        ".as-oil",
        """[data-cypress="soft-push-notification-modal"]""",
      ),
      "adGuardBase" -> js.Array(
        ".BetterJsPopOverlay",
        fromB64("I2FkXzMwMFgyNTA="),
        fromB64("I2Jhbm5lcmZsb2F0MjI="),
        fromB64("I2NhbXBhaWduLWJhbm5lcg=="),
        fromB64("I0FkLUNvbnRlbnQ="),
      ),
      "adGuardChinese" -> js.Array(
        fromB64("LlppX2FkX2FfSA=="),
        fromB64("YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd"),
        "#widget-quan",
        fromB64("YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd"),
        fromB64("YVtocmVmKj0iLjE5NTZobC5jb20vIl0="),
      ),
      "adGuardFrench" -> js.Array(
        "#pavePub",
        fromB64("LmFkLWRlc2t0b3AtcmVjdGFuZ2xl"),
        ".mobile_adhesion",
        ".widgetadv",
        fromB64("LmFkc19iYW4="),
      ),
      "adGuardGerman" -> js.Array("""aside[data-portal-id="leaderboard"]"""),
      "adGuardJapanese" -> js.Array(
        "#kauli_yad_1",
        fromB64("YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0="),
        fromB64("Ll9wb3BJbl9pbmZpbml0ZV9hZA=="),
        fromB64("LmFkZ29vZ2xl"),
        fromB64("Ll9faXNib29zdFJldHVybkFk"),
      ),
      "adGuardMobile" -> js.Array(
        fromB64("YW1wLWF1dG8tYWRz"),
        fromB64("LmFtcF9hZA=="),
        """amp-embed[type="24smi"]""",
        "#mgid_iframe1",
        fromB64("I2FkX2ludmlld19hcmVh"),
      ),
      "adGuardRussian" -> js.Array(
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0="),
        fromB64("LnJlY2xhbWE="),
        """div[id^="smi2adblock"]""",
        fromB64("ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd"),
        "#psyduckpockeball",
      ),
      "adGuardSocial" -> js.Array(
        fromB64("YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0="),
        fromB64("YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0="),
        ".etsy-tweet",
        "#inlineShare",
        ".popup-social",
      ),
      "adGuardSpanishPortuguese" -> js.Array("#barraPublicidade", "#Publicidade", "#publiEspecial", "#queTooltip", ".cnt-publi"),
      "adGuardTrackingProtection" -> js.Array(
        "#qoo-counter",
        fromB64("YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=="),
        fromB64("YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0="),
        fromB64("YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=="),
        "#top100counter",
      ),
      "adGuardTurkish" -> js.Array(
        "#backkapat",
        fromB64("I3Jla2xhbWk="),
        fromB64("YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0="),
        fromB64("YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd"),
        fromB64("YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ=="),
      ),
      "bulgarian" -> js.Array(fromB64("dGQjZnJlZW5ldF90YWJsZV9hZHM="), "#ea_intext_div", ".lapni-pop-over", "#xenium_hot_offers"),
      "easyList" -> js.Array(
        ".yb-floorad",
        fromB64("LndpZGdldF9wb19hZHNfd2lkZ2V0"),
        fromB64("LnRyYWZmaWNqdW5reS1hZA=="),
        ".textad_headline",
        fromB64("LnNwb25zb3JlZC10ZXh0LWxpbmtz"),
      ),
      "easyListChina" -> js.Array(
        fromB64("LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=="),
        fromB64("LmZyb250cGFnZUFkdk0="),
        "#taotaole",
        "#aafoot.top_box",
        ".cfa_popup",
      ),
      "easyListCookie" -> js.Array(
        ".ezmob-footer",
        ".cc-CookieWarning",
        "[data-cookie-number]",
        fromB64("LmF3LWNvb2tpZS1iYW5uZXI="),
        ".sygnal24-gdpr-modal-wrap",
      ),
      "easyListCzechSlovak" -> js.Array(
        "#onlajny-stickers",
        fromB64("I3Jla2xhbW5pLWJveA=="),
        fromB64("LnJla2xhbWEtbWVnYWJvYXJk"),
        ".sklik",
        fromB64("W2lkXj0ic2tsaWtSZWtsYW1hIl0="),
      ),
      "easyListDutch" -> js.Array(
        fromB64("I2FkdmVydGVudGll"),
        fromB64("I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=="),
        ".adstekst",
        fromB64("YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0="),
        "#semilo-lrectangle",
      ),
      "easyListGermany" -> js.Array(
        "#SSpotIMPopSlider",
        fromB64("LnNwb25zb3JsaW5rZ3J1ZW4="),
        fromB64("I3dlcmJ1bmdza3k="),
        fromB64("I3Jla2xhbWUtcmVjaHRzLW1pdHRl"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0="),
      ),
      "easyListItaly" -> js.Array(
        fromB64("LmJveF9hZHZfYW5udW5jaQ=="),
        ".sb-box-pubbliredazionale",
        fromB64("YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ=="),
      ),
      "easyListLithuania" -> js.Array(
        fromB64("LnJla2xhbW9zX3RhcnBhcw=="),
        fromB64("LnJla2xhbW9zX251b3JvZG9z"),
        fromB64("aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd"),
        fromB64("aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd"),
        fromB64("aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd"),
      ),
      "estonian" -> js.Array(fromB64("QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ==")),
      "fanboyAnnoyances" -> js.Array("#ac-lre-player", ".navigate-to-top", "#subscribe_popup", ".newsletter_holder", "#back-top"),
      "fanboyAntiFacebook" -> js.Array(".util-bar-module-firefly-visible"),
      "fanboyEnhancedTrackers" -> js.Array(
        ".open.pushModal",
        "#issuem-leaky-paywall-articles-zero-remaining-nag",
        "#sovrn_container",
        """div[class$="-hide"][zoompage-fontsize][style="display: block;"]""",
        ".BlockNag__Card",
      ),
      "fanboySocial" -> js.Array("#FollowUs", "#meteored_share", "#social_follow", ".article-sharer", ".community__social-desc"),
      "frellwitSwedish" -> js.Array(
        fromB64("YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=="),
        fromB64("YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=="),
        "article.category-samarbete",
        fromB64("ZGl2LmhvbGlkQWRz"),
        "ul.adsmodern",
      ),
      "greekAdBlock" -> js.Array(
        fromB64("QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd"),
        fromB64("QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=="),
        fromB64("QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd"),
        "DIV.agores300",
        "TABLE.advright",
      ),
      "hungarian" -> js.Array(
        "#cemp_doboz",
        ".optimonk-iframe-container",
        fromB64("LmFkX19tYWlu"),
        fromB64("W2NsYXNzKj0iR29vZ2xlQWRzIl0="),
        "#hirdetesek_box",
      ),
      "iDontCareAboutCookies" -> js.Array(
        """.alert-info[data-block-track*="CookieNotice"]""",
        ".ModuleTemplateCookieIndicator",
        ".o--cookies--container",
        "#cookies-policy-sticky",
        "#stickyCookieBar",
      ),
      "icelandicAbp" -> js.Array(fromB64("QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ==")),
      "latvian" -> js.Array(
        fromB64(
          "YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0O" +
            "iA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0=",
        ),
        fromB64(
          "YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6I" +
            "DMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ==",
        ),
      ),
      "listKr" -> js.Array(
        fromB64("YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0="),
        fromB64("I2xpdmVyZUFkV3JhcHBlcg=="),
        fromB64("YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=="),
        fromB64("aW5zLmZhc3R2aWV3LWFk"),
        ".revenue_unit_item.dable",
      ),
      "listeAr" -> js.Array(
        fromB64("LmdlbWluaUxCMUFk"),
        ".right-and-left-sponsers",
        fromB64("YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=="),
        fromB64("YVtocmVmKj0iYm9vcmFxLm9yZyJd"),
        fromB64("YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd"),
      ),
      "listeFr" -> js.Array(
        fromB64("YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=="),
        fromB64("I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=="),
        fromB64("YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0="),
        ".site-pub-interstitiel",
        """div[id^="crt-"][data-criteo-id]""",
      ),
      "officialPolish" -> js.Array(
        "#ceneo-placeholder-ceneo-12",
        fromB64("W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd"),
        fromB64("YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=="),
        fromB64("YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=="),
        fromB64("ZGl2I3NrYXBpZWNfYWQ="),
      ),
      "ro" -> js.Array(
        fromB64("YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd"),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0="),
        fromB64("YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd"),
        """a[href^="/url/"]""",
      ),
      "ruAd" -> js.Array(
        fromB64("YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd"),
        fromB64("YVtocmVmKj0iLy91dGltZy5ydS8iXQ=="),
        fromB64("YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0="),
        "#pgeldiz",
        ".yandex-rtb-block",
      ),
      "thaiAds" -> js.Array(
        "a[href*=macau-uta-popup]",
        fromB64("I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=="),
        fromB64("LmFkczMwMHM="),
        ".bumq",
        ".img-kosana",
      ),
      "webAnnoyancesUltralist" -> js.Array(
        "#mod-social-share-2",
        "#social-tools",
        fromB64("LmN0cGwtZnVsbGJhbm5lcg=="),
        ".zergnet-recommend",
        ".yt.btn-link.btn-md.btn",
      ),
    )
  }

  def isApplicable(): Boolean = Browser.isWebKit() || Browser.isAndroid()

  def getBlockedSelectors(selectors: js.Array[String]): js.Promise[js.Dictionary[Boolean]] = {
    val d = Host.document
    val root = d.createElement("div")
    val elements = new js.Array[js.Dynamic](selectors.length)
    val blockedSelectors = js.Dictionary.empty[Boolean]
    forceShow(root)

    var i = 0
    while (i < selectors.length) {
      val element = Dom.selectorToElement(selectors(i))
      if (Host.toJsString(element.tagName.asInstanceOf[js.Any]) == "DIALOG")
        element.show()
      val holder = d.createElement("div")
      forceShow(holder)
      holder.appendChild(element)
      root.appendChild(holder)
      elements(i) = element
      i += 1
    }

    def waitBody(): js.Promise[Unit] =
      if (Host.jsTruthy(d.body.asInstanceOf[js.Any])) js.Promise.resolve(())
      else Async.wait(50).`then`(_ => waitBody())

    waitBody().`then` { (_: Unit) =>
      d.body.appendChild(root)
      try {
        var j = 0
        while (j < selectors.length) {
          if (!Host.jsTruthy(elements(j).offsetParent.asInstanceOf[js.Any]))
            blockedSelectors(selectors(j)) = true
          j += 1
        }
      } finally {
        val parent = root.parentNode
        if (Host.jsTruthy(parent.asInstanceOf[js.Any])) parent.removeChild(root)
      }
      blockedSelectors
    }
  }

  private def forceShow(element: js.Dynamic): Unit = {
    element.style.setProperty("visibility", "hidden", "important")
    element.style.setProperty("display", "block", "important")
  }

  private def printDebug(filters: js.Dictionary[js.Array[String]], blockedSelectors: js.Dictionary[Boolean]): Unit = {
    var message = "DOM blockers debug:\n```"
    val filterNames = js.Object.keys(filters.asInstanceOf[js.Object])
    var i = 0
    while (i < filterNames.length) {
      val filterName = filterNames(i)
      message += s"\n$filterName:"
      val selectors = filters(filterName)
      var j = 0
      while (j < selectors.length) {
        val selector = selectors(j)
        val mark = if (blockedSelectors.get(selector).contains(true)) "🚫" else "➡️"
        message += s"\n  $mark $selector"
        j += 1
      }
      i += 1
    }
    Host.consoleLog(s"$message\n```")
  }

  def getDomBlockers(options: js.Any): js.Promise[js.Any] = {
    if (!isApplicable()) return js.Promise.resolve[js.Any](js.undefined)
    val debug = Host.jsTruthy(Host.get(options, "debug"))
    val filters = getFilters()
    val filterNames = js.Object.keys(filters.asInstanceOf[js.Object])
    val allSelectors = js.Array[String]()
    var i = 0
    while (i < filterNames.length) {
      allSelectors.push(filters(filterNames(i)).toSeq*)
      i += 1
    }
    getBlockedSelectors(allSelectors).`then` { (blockedSelectors: js.Dictionary[Boolean]) =>
      if (debug) printDebug(filters, blockedSelectors)
      val activeBlockers = filterNames.filter { filterName =>
        val selectors = filters(filterName)
        var blockedCount = 0
        var j = 0
        while (j < selectors.length) {
          if (blockedSelectors.get(selectors(j)).contains(true)) blockedCount += 1
          j += 1
        }
        blockedCount > selectors.length * 0.6
      }
      activeBlockers.sort()
      activeBlockers.asInstanceOf[js.Any]
    }
  }

  val source: js.Function1[js.Any, js.Promise[js.Any]] = getDomBlockers
}
