internal fun getFilters(): dynamic {
    val fromB64 = { value: String -> jsAtob(value) }
    val filters = obj()
    fun add(name: String, vararg selectors: String) {
        val list = arr()
        for (selector in selectors) jsPush(list, selector)
        jsSet(filters, name, list)
    }
    add("abpIndo", "#Iklan-Melayang", "#Kolom-Iklan-728", "#SidebarIklan-wrapper", "[title=\"ALIENBOLA\" i]", fromB64("I0JveC1CYW5uZXItYWRz"))
    add("abpvn", ".quangcao", "#mobileCatfish", fromB64("LmNsb3NlLWFkcw=="), "[id^=\"bn_bottom_fixed_\"]", "#pmadv")
    add("adBlockFinland", ".mainostila", fromB64("LnNwb25zb3JpdA=="), ".ylamainos", fromB64("YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd"))
    add("adBlockPersian", "#navbar_notice_50", ".kadr", "TABLE[width=\"140px\"]", "#divAgahi", fromB64("YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd"))
    add("adBlockWarningRemoval", "#adblock-honeypot", ".adblocker-root", ".wp_adblock_detect", fromB64("LmhlYWRlci1ibG9ja2VkLWFk"), fromB64("I2FkX2Jsb2NrZXI="))
    add("adGuardAnnoyances", ".hs-sosyal", "#cookieconsentdiv", "div[class^=\"app_gdpr\"]", ".as-oil", "[data-cypress=\"soft-push-notification-modal\"]")
    add("adGuardBase", ".BetterJsPopOverlay", fromB64("I2FkXzMwMFgyNTA="), fromB64("I2Jhbm5lcmZsb2F0MjI="), fromB64("I2NhbXBhaWduLWJhbm5lcg=="), fromB64("I0FkLUNvbnRlbnQ="))
    add("adGuardChinese", fromB64("LlppX2FkX2FfSA=="), fromB64("YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd"), "#widget-quan", fromB64("YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd"), fromB64("YVtocmVmKj0iLjE5NTZobC5jb20vIl0="))
    add("adGuardFrench", "#pavePub", fromB64("LmFkLWRlc2t0b3AtcmVjdGFuZ2xl"), ".mobile_adhesion", ".widgetadv", fromB64("LmFkc19iYW4="))
    add("adGuardGerman", "aside[data-portal-id=\"leaderboard\"]")
    add("adGuardJapanese", "#kauli_yad_1", fromB64("YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0="), fromB64("Ll9wb3BJbl9pbmZpbml0ZV9hZA=="), fromB64("LmFkZ29vZ2xl"), fromB64("Ll9faXNib29zdFJldHVybkFk"))
    add("adGuardMobile", fromB64("YW1wLWF1dG8tYWRz"), fromB64("LmFtcF9hZA=="), "amp-embed[type=\"24smi\"]", "#mgid_iframe1", fromB64("I2FkX2ludmlld19hcmVh"))
    add("adGuardRussian", fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0="), fromB64("LnJlY2xhbWE="), "div[id^=\"smi2adblock\"]", fromB64("ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd"), "#psyduckpockeball")
    add("adGuardSocial", fromB64("YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0="), fromB64("YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0="), ".etsy-tweet", "#inlineShare", ".popup-social")
    add("adGuardSpanishPortuguese", "#barraPublicidade", "#Publicidade", "#publiEspecial", "#queTooltip", ".cnt-publi")
    add("adGuardTrackingProtection", "#qoo-counter", fromB64("YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=="), fromB64("YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0="), fromB64("YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=="), "#top100counter")
    add("adGuardTurkish", "#backkapat", fromB64("I3Jla2xhbWk="), fromB64("YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0="), fromB64("YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd"), fromB64("YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ=="))
    add("bulgarian", fromB64("dGQjZnJlZW5ldF90YWJsZV9hZHM="), "#ea_intext_div", ".lapni-pop-over", "#xenium_hot_offers")
    add("easyList", ".yb-floorad", fromB64("LndpZGdldF9wb19hZHNfd2lkZ2V0"), fromB64("LnRyYWZmaWNqdW5reS1hZA=="), ".textad_headline", fromB64("LnNwb25zb3JlZC10ZXh0LWxpbmtz"))
    add("easyListChina", fromB64("LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=="), fromB64("LmZyb250cGFnZUFkdk0="), "#taotaole", "#aafoot.top_box", ".cfa_popup")
    add("easyListCookie", ".ezmob-footer", ".cc-CookieWarning", "[data-cookie-number]", fromB64("LmF3LWNvb2tpZS1iYW5uZXI="), ".sygnal24-gdpr-modal-wrap")
    add("easyListCzechSlovak", "#onlajny-stickers", fromB64("I3Jla2xhbW5pLWJveA=="), fromB64("LnJla2xhbWEtbWVnYWJvYXJk"), ".sklik", fromB64("W2lkXj0ic2tsaWtSZWtsYW1hIl0="))
    add("easyListDutch", fromB64("I2FkdmVydGVudGll"), fromB64("I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=="), ".adstekst", fromB64("YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0="), "#semilo-lrectangle")
    add("easyListGermany", "#SSpotIMPopSlider", fromB64("LnNwb25zb3JsaW5rZ3J1ZW4="), fromB64("I3dlcmJ1bmdza3k="), fromB64("I3Jla2xhbWUtcmVjaHRzLW1pdHRl"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0="))
    add("easyListItaly", fromB64("LmJveF9hZHZfYW5udW5jaQ=="), ".sb-box-pubbliredazionale", fromB64("YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ=="))
    add("easyListLithuania", fromB64("LnJla2xhbW9zX3RhcnBhcw=="), fromB64("LnJla2xhbW9zX251b3JvZG9z"), fromB64("aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd"), fromB64("aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd"), fromB64("aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd"))
    add("estonian", fromB64("QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ=="))
    add("fanboyAnnoyances", "#ac-lre-player", ".navigate-to-top", "#subscribe_popup", ".newsletter_holder", "#back-top")
    add("fanboyAntiFacebook", ".util-bar-module-firefly-visible")
    add("fanboyEnhancedTrackers", ".open.pushModal", "#issuem-leaky-paywall-articles-zero-remaining-nag", "#sovrn_container", "div[class$=\"-hide\"][zoompage-fontsize][style=\"display: block;\"]", ".BlockNag__Card")
    add("fanboySocial", "#FollowUs", "#meteored_share", "#social_follow", ".article-sharer", ".community__social-desc")
    add("frellwitSwedish", fromB64("YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=="), fromB64("YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=="), "article.category-samarbete", fromB64("ZGl2LmhvbGlkQWRz"), "ul.adsmodern")
    add("greekAdBlock", fromB64("QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd"), fromB64("QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=="), fromB64("QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd"), "DIV.agores300", "TABLE.advright")
    add("hungarian", "#cemp_doboz", ".optimonk-iframe-container", fromB64("LmFkX19tYWlu"), fromB64("W2NsYXNzKj0iR29vZ2xlQWRzIl0="), "#hirdetesek_box")
    add("iDontCareAboutCookies", ".alert-info[data-block-track*=\"CookieNotice\"]", ".ModuleTemplateCookieIndicator", ".o--cookies--container", "#cookies-policy-sticky", "#stickyCookieBar")
    add("icelandicAbp", fromB64("QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ=="))
    add(
        "latvian",
        fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0OiA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0="),
        fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6IDMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ=="),
    )
    add("listKr", fromB64("YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0="), fromB64("I2xpdmVyZUFkV3JhcHBlcg=="), fromB64("YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=="), fromB64("aW5zLmZhc3R2aWV3LWFk"), ".revenue_unit_item.dable")
    add("listeAr", fromB64("LmdlbWluaUxCMUFk"), ".right-and-left-sponsers", fromB64("YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=="), fromB64("YVtocmVmKj0iYm9vcmFxLm9yZyJd"), fromB64("YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd"))
    add("listeFr", fromB64("YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=="), fromB64("I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=="), fromB64("YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0="), ".site-pub-interstitiel", "div[id^=\"crt-\"][data-criteo-id]")
    add("officialPolish", "#ceneo-placeholder-ceneo-12", fromB64("W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd"), fromB64("YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=="), fromB64("YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=="), fromB64("ZGl2I3NrYXBpZWNfYWQ="))
    add("ro", fromB64("YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd"), fromB64("YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0="), fromB64("YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd"), "a[href^=\"/url/\"]")
    add("ruAd", fromB64("YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd"), fromB64("YVtocmVmKj0iLy91dGltZy5ydS8iXQ=="), fromB64("YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0="), "#pgeldiz", ".yandex-rtb-block")
    add("thaiAds", "a[href*=macau-uta-popup]", fromB64("I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=="), fromB64("LmFkczMwMHM="), ".bumq", ".img-kosana")
    add("webAnnoyancesUltralist", "#mod-social-share-2", "#social-tools", fromB64("LmN0cGwtZnVsbGJhbm5lcg=="), ".zergnet-recommend", ".yt.btn-link.btn-md.btn")
    return filters
}

internal fun isDomBlockersApplicable(): Boolean = isWebKit() || isAndroid()

internal fun getDomBlockers(options: dynamic): dynamic {
    if (!isDomBlockersApplicable()) {
        return jsPromiseResolve(jsUndefined)
    }
    val debug = isTruthy(options?.debug)
    val filters = getFilters()
    val filterNames = jsKeys(filters)
    val allSelectors = arr()
    var i = 0
    while (i < jsLength(filterNames)) {
        val name = jsString(jsIndex(filterNames, i))
        val selectors = jsGet(filters, name)
        var j = 0
        while (j < jsLength(selectors)) {
            jsPush(allSelectors, jsIndex(selectors, j))
            j += 1
        }
        i += 1
    }
    return getBlockedSelectors(allSelectors).then({ blockedSelectors: dynamic ->
        if (debug) printDomBlockerDebug(filters, blockedSelectors)
        val activeBlockers = arr()
        var n = 0
        while (n < jsLength(filterNames)) {
            val filterName = jsString(jsIndex(filterNames, n))
            val selectors = jsGet(filters, filterName)
            var blockedCount = 0
            var s = 0
            while (s < jsLength(selectors)) {
                if (isTruthy(jsGet(blockedSelectors, jsString(jsIndex(selectors, s))))) blockedCount += 1
                s += 1
            }
            if (blockedCount > jsLength(selectors) * 0.6) {
                jsPush(activeBlockers, filterName)
            }
            n += 1
        }
        jsSort(activeBlockers)
        activeBlockers
    })
}

internal fun getBlockedSelectors(selectors: dynamic): dynamic {
    val d = jsDocument()
    val root = d.createElement("div")
    val elements = arr()
    val blockedSelectors = obj()
    forceShow(root)
    var i = 0
    while (i < jsLength(selectors)) {
        val element = selectorToElement(jsString(jsIndex(selectors, i)))
        if (element.tagName == "DIALOG") {
            element.show()
        }
        val holder = d.createElement("div")
        forceShow(holder)
        holder.appendChild(element)
        root.appendChild(holder)
        jsPush(elements, element)
        i += 1
    }

    fun waitForBody(): dynamic {
        return if (isTruthy(d.body)) jsPromiseResolve(jsUndefined) else wait(50.0).then({ waitForBody() })
    }

    return waitForBody().then({
        d.body.appendChild(root)
        val measured = jsTry {
            var index = 0
            while (index < jsLength(selectors)) {
                if (!isTruthy(jsIndex(elements, index).offsetParent)) {
                    jsSet(blockedSelectors, jsString(jsIndex(selectors, index)), true)
                }
                index += 1
            }
            jsUndefined
        }
        val parent = root.parentNode
        if (isTruthy(parent)) parent.removeChild(root)
        if (!jsTryOk(measured)) jsThrow(measured.error)
        blockedSelectors
    })
}

private fun forceShow(element: dynamic) {
    element.style.setProperty("visibility", "hidden", "important")
    element.style.setProperty("display", "block", "important")
}

private fun printDomBlockerDebug(filters: dynamic, blockedSelectors: dynamic) {
    var message = "DOM blockers debug:\n```"
    val names = jsKeys(filters)
    var i = 0
    while (i < jsLength(names)) {
        val filterName = jsString(jsIndex(names, i))
        message += "\n$filterName:"
        val selectors = jsGet(filters, filterName)
        var j = 0
        while (j < jsLength(selectors)) {
            val selector = jsString(jsIndex(selectors, j))
            message += "\n  ${if (isTruthy(jsGet(blockedSelectors, selector))) "🚫" else "➡️"} $selector"
            j += 1
        }
        i += 1
    }
    jsConsoleLog("$message\n```")
}
