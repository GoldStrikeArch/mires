package main

import "github.com/gopherjs/gopherjs/js"

// filter is a content blocker filter list and the selectors that represent it.
type filter struct {
	name      string
	selectors []string
}

// getFilters lists single-element selectors (no operators like space, + or >). embed and
// position: fixed always count as blocked because they have no offsetParent; iframe and
// [src=] are avoided because they produce extra HTTP requests. The "inappropriate"
// selectors are base64-encoded (issue 734) and decoded with the host atob. A function is
// used instead of a variable to help tree-shaking. The list is generated; see
// docs/content_blockers.md in the FingerprintJS repository.
func getFilters() []filter {
	fromB64 := func(encoded string) string { return js.Global.Call("atob", encoded).String() }

	return []filter{
		{"abpIndo", []string{
			"#Iklan-Melayang",
			"#Kolom-Iklan-728",
			"#SidebarIklan-wrapper",
			`[title="ALIENBOLA" i]`,
			fromB64("I0JveC1CYW5uZXItYWRz"),
		}},
		{"abpvn", []string{".quangcao", "#mobileCatfish", fromB64("LmNsb3NlLWFkcw=="), `[id^="bn_bottom_fixed_"]`, "#pmadv"}},
		{"adBlockFinland", []string{
			".mainostila",
			fromB64("LnNwb25zb3JpdA=="),
			".ylamainos",
			fromB64("YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd"),
		}},
		{"adBlockPersian", []string{
			"#navbar_notice_50",
			".kadr",
			`TABLE[width="140px"]`,
			"#divAgahi",
			fromB64("YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd"),
		}},
		{"adBlockWarningRemoval", []string{
			"#adblock-honeypot",
			".adblocker-root",
			".wp_adblock_detect",
			fromB64("LmhlYWRlci1ibG9ja2VkLWFk"),
			fromB64("I2FkX2Jsb2NrZXI="),
		}},
		{"adGuardAnnoyances", []string{
			".hs-sosyal",
			"#cookieconsentdiv",
			`div[class^="app_gdpr"]`,
			".as-oil",
			`[data-cypress="soft-push-notification-modal"]`,
		}},
		{"adGuardBase", []string{
			".BetterJsPopOverlay",
			fromB64("I2FkXzMwMFgyNTA="),
			fromB64("I2Jhbm5lcmZsb2F0MjI="),
			fromB64("I2NhbXBhaWduLWJhbm5lcg=="),
			fromB64("I0FkLUNvbnRlbnQ="),
		}},
		{"adGuardChinese", []string{
			fromB64("LlppX2FkX2FfSA=="),
			fromB64("YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd"),
			"#widget-quan",
			fromB64("YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd"),
			fromB64("YVtocmVmKj0iLjE5NTZobC5jb20vIl0="),
		}},
		{"adGuardFrench", []string{
			"#pavePub",
			fromB64("LmFkLWRlc2t0b3AtcmVjdGFuZ2xl"),
			".mobile_adhesion",
			".widgetadv",
			fromB64("LmFkc19iYW4="),
		}},
		{"adGuardGerman", []string{`aside[data-portal-id="leaderboard"]`}},
		{"adGuardJapanese", []string{
			"#kauli_yad_1",
			fromB64("YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0="),
			fromB64("Ll9wb3BJbl9pbmZpbml0ZV9hZA=="),
			fromB64("LmFkZ29vZ2xl"),
			fromB64("Ll9faXNib29zdFJldHVybkFk"),
		}},
		{"adGuardMobile", []string{
			fromB64("YW1wLWF1dG8tYWRz"),
			fromB64("LmFtcF9hZA=="),
			`amp-embed[type="24smi"]`,
			"#mgid_iframe1",
			fromB64("I2FkX2ludmlld19hcmVh"),
		}},
		{"adGuardRussian", []string{
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0="),
			fromB64("LnJlY2xhbWE="),
			`div[id^="smi2adblock"]`,
			fromB64("ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd"),
			"#psyduckpockeball",
		}},
		{"adGuardSocial", []string{
			fromB64("YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0="),
			fromB64("YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0="),
			".etsy-tweet",
			"#inlineShare",
			".popup-social",
		}},
		{"adGuardSpanishPortuguese", []string{"#barraPublicidade", "#Publicidade", "#publiEspecial", "#queTooltip", ".cnt-publi"}},
		{"adGuardTrackingProtection", []string{
			"#qoo-counter",
			fromB64("YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=="),
			fromB64("YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0="),
			fromB64("YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=="),
			"#top100counter",
		}},
		{"adGuardTurkish", []string{
			"#backkapat",
			fromB64("I3Jla2xhbWk="),
			fromB64("YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0="),
			fromB64("YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd"),
			fromB64("YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ=="),
		}},
		{"bulgarian", []string{fromB64("dGQjZnJlZW5ldF90YWJsZV9hZHM="), "#ea_intext_div", ".lapni-pop-over", "#xenium_hot_offers"}},
		{"easyList", []string{
			".yb-floorad",
			fromB64("LndpZGdldF9wb19hZHNfd2lkZ2V0"),
			fromB64("LnRyYWZmaWNqdW5reS1hZA=="),
			".textad_headline",
			fromB64("LnNwb25zb3JlZC10ZXh0LWxpbmtz"),
		}},
		{"easyListChina", []string{
			fromB64("LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=="),
			fromB64("LmZyb250cGFnZUFkdk0="),
			"#taotaole",
			"#aafoot.top_box",
			".cfa_popup",
		}},
		{"easyListCookie", []string{
			".ezmob-footer",
			".cc-CookieWarning",
			"[data-cookie-number]",
			fromB64("LmF3LWNvb2tpZS1iYW5uZXI="),
			".sygnal24-gdpr-modal-wrap",
		}},
		{"easyListCzechSlovak", []string{
			"#onlajny-stickers",
			fromB64("I3Jla2xhbW5pLWJveA=="),
			fromB64("LnJla2xhbWEtbWVnYWJvYXJk"),
			".sklik",
			fromB64("W2lkXj0ic2tsaWtSZWtsYW1hIl0="),
		}},
		{"easyListDutch", []string{
			fromB64("I2FkdmVydGVudGll"),
			fromB64("I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=="),
			".adstekst",
			fromB64("YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0="),
			"#semilo-lrectangle",
		}},
		{"easyListGermany", []string{
			"#SSpotIMPopSlider",
			fromB64("LnNwb25zb3JsaW5rZ3J1ZW4="),
			fromB64("I3dlcmJ1bmdza3k="),
			fromB64("I3Jla2xhbWUtcmVjaHRzLW1pdHRl"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0="),
		}},
		{"easyListItaly", []string{
			fromB64("LmJveF9hZHZfYW5udW5jaQ=="),
			".sb-box-pubbliredazionale",
			fromB64("YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ=="),
		}},
		{"easyListLithuania", []string{
			fromB64("LnJla2xhbW9zX3RhcnBhcw=="),
			fromB64("LnJla2xhbW9zX251b3JvZG9z"),
			fromB64("aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd"),
			fromB64("aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd"),
			fromB64("aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd"),
		}},
		{"estonian", []string{fromB64("QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ==")}},
		{"fanboyAnnoyances", []string{"#ac-lre-player", ".navigate-to-top", "#subscribe_popup", ".newsletter_holder", "#back-top"}},
		{"fanboyAntiFacebook", []string{".util-bar-module-firefly-visible"}},
		{"fanboyEnhancedTrackers", []string{
			".open.pushModal",
			"#issuem-leaky-paywall-articles-zero-remaining-nag",
			"#sovrn_container",
			`div[class$="-hide"][zoompage-fontsize][style="display: block;"]`,
			".BlockNag__Card",
		}},
		{"fanboySocial", []string{"#FollowUs", "#meteored_share", "#social_follow", ".article-sharer", ".community__social-desc"}},
		{"frellwitSwedish", []string{
			fromB64("YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=="),
			fromB64("YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=="),
			"article.category-samarbete",
			fromB64("ZGl2LmhvbGlkQWRz"),
			"ul.adsmodern",
		}},
		{"greekAdBlock", []string{
			fromB64("QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd"),
			fromB64("QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=="),
			fromB64("QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd"),
			"DIV.agores300",
			"TABLE.advright",
		}},
		{"hungarian", []string{
			"#cemp_doboz",
			".optimonk-iframe-container",
			fromB64("LmFkX19tYWlu"),
			fromB64("W2NsYXNzKj0iR29vZ2xlQWRzIl0="),
			"#hirdetesek_box",
		}},
		{"iDontCareAboutCookies", []string{
			`.alert-info[data-block-track*="CookieNotice"]`,
			".ModuleTemplateCookieIndicator",
			".o--cookies--container",
			"#cookies-policy-sticky",
			"#stickyCookieBar",
		}},
		{"icelandicAbp", []string{fromB64("QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ==")}},
		{"latvian", []string{
			fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0O" +
				"iA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0="),
			fromB64("YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6I" +
				"DMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ=="),
		}},
		{"listKr", []string{
			fromB64("YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0="),
			fromB64("I2xpdmVyZUFkV3JhcHBlcg=="),
			fromB64("YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=="),
			fromB64("aW5zLmZhc3R2aWV3LWFk"),
			".revenue_unit_item.dable",
		}},
		{"listeAr", []string{
			fromB64("LmdlbWluaUxCMUFk"),
			".right-and-left-sponsers",
			fromB64("YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=="),
			fromB64("YVtocmVmKj0iYm9vcmFxLm9yZyJd"),
			fromB64("YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd"),
		}},
		{"listeFr", []string{
			fromB64("YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=="),
			fromB64("I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=="),
			fromB64("YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0="),
			".site-pub-interstitiel",
			`div[id^="crt-"][data-criteo-id]`,
		}},
		{"officialPolish", []string{
			"#ceneo-placeholder-ceneo-12",
			fromB64("W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd"),
			fromB64("YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=="),
			fromB64("YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=="),
			fromB64("ZGl2I3NrYXBpZWNfYWQ="),
		}},
		{"ro", []string{
			fromB64("YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd"),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0="),
			fromB64("YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd"),
			`a[href^="/url/"]`,
		}},
		{"ruAd", []string{
			fromB64("YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd"),
			fromB64("YVtocmVmKj0iLy91dGltZy5ydS8iXQ=="),
			fromB64("YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0="),
			"#pgeldiz",
			".yandex-rtb-block",
		}},
		{"thaiAds", []string{
			"a[href*=macau-uta-popup]",
			fromB64("I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=="),
			fromB64("LmFkczMwMHM="),
			".bumq",
			".img-kosana",
		}},
		{"webAnnoyancesUltralist", []string{
			"#mod-social-share-2",
			"#social-tools",
			fromB64("LmN0cGwtZnVsbGJhbm5lcg=="),
			".zergnet-recommend",
			".yt.btn-link.btn-md.btn",
		}},
	}
}

// getDomBlockers lists the active content blockers, sorted alphabetically. The source is
// slightly unstable: Safari can disable all content blockers of a page with 2 taps, and of
// a domain permanently with 4, so an empty list means "no signal" rather than "no blockers".
func getDomBlockers(options *js.Object) *js.Object {
	return async(func(resolve, reject *js.Object) {
		debug := js.Undefined
		if options != js.Undefined {
			debug = options.Get("debug")
		}

		if !isDomBlockersApplicable() {
			resolve.Invoke(js.Undefined)
			return
		}

		filters := getFilters()
		var allSelectors []string
		for _, f := range filters {
			allSelectors = append(allSelectors, f.selectors...)
		}

		blockedSelectors, found := getBlockedSelectors(allSelectors)
		await(found, reject, func(*js.Object) {
			if debug.Bool() {
				printDomBlockersDebug(filters, blockedSelectors)
			}

			activeBlockers := array()
			for _, f := range filters {
				blockedCount := 0
				for _, selector := range f.selectors {
					if blockedSelectors[selector] {
						blockedCount++
					}
				}
				if float64(blockedCount) > float64(len(f.selectors))*0.6 {
					activeBlockers.Call("push", f.name)
				}
			}
			activeBlockers.Call("sort")

			resolve.Invoke(activeBlockers)
		})
	})
}

// isDomBlockersApplicable: Safari (desktop and mobile) and all Android browsers keep content
// blockers in both regular and private mode.
func isDomBlockersApplicable() bool { return isWebKit() || isAndroid() }

// getBlockedSelectors finds which of the selectors the page's content blockers hide. The
// blocked set is complete once done fulfils.
func getBlockedSelectors(selectors []string) (blockedSelectors map[string]bool, done *js.Object) {
	blockedSelectors = map[string]bool{}
	done = async(func(resolve, reject *js.Object) {
		d := document()
		root := d.Call("createElement", "div")
		elements := make([]*js.Object, len(selectors))

		forceShow(root)

		// Create all the elements that can be blocked first: alternating tree modification
		// and layout reading is very slow.
		for i, selector := range selectors {
			element := selectorToElement(selector)
			if equals(element.Get("tagName"), "DIALOG") {
				element.Call("show")
			}
			holder := d.Call("createElement", "div") // Protects from unwanted effects of + and ~ selectors
			forceShow(holder)
			holder.Call("appendChild", element)
			root.Call("appendChild", holder)
			elements[i] = element
		}

		// document.body can be null while the page is loading
		awaitUntil(func() bool { return d.Get("body").Bool() }, defaultDomPollInterval, reject, func() {
			d.Get("body").Call("appendChild", root)

			exception, threw := tryJS(func() {
				// Then check which of the elements are blocked
				for i, selector := range selectors {
					if !elements[i].Get("offsetParent").Bool() {
						blockedSelectors[selector] = true
					}
				}
			})

			// Then remove the elements
			if parent := root.Get("parentNode"); !isNullish(parent) {
				parent.Call("removeChild", root)
			}
			if threw {
				throw(exception)
			}

			resolve.Invoke()
		})
	})
	return blockedSelectors, done
}

func forceShow(element *js.Object) {
	style := element.Get("style")
	style.Call("setProperty", "visibility", "hidden", "important")
	style.Call("setProperty", "display", "block", "important")
}

func printDomBlockersDebug(filters []filter, blockedSelectors map[string]bool) {
	message := "DOM blockers debug:\n```"
	for _, f := range filters {
		message += "\n" + f.name + ":"
		for _, selector := range f.selectors {
			mark := "➡️"
			if blockedSelectors[selector] {
				mark = "🚫"
			}
			message += "\n  " + mark + " " + selector
		}
	}
	console().Call("log", message+"\n```")
}
