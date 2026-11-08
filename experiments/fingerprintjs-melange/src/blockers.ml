open Host

let get_filters () : js =
  [%mel.raw
    {|
    (function getFilters() {
      var fromB64 = atob;
      return {
        abpIndo: ['#Iklan-Melayang', '#Kolom-Iklan-728', '#SidebarIklan-wrapper', '[title="ALIENBOLA" i]', fromB64('I0JveC1CYW5uZXItYWRz')],
        abpvn: ['.quangcao', '#mobileCatfish', fromB64('LmNsb3NlLWFkcw=='), '[id^="bn_bottom_fixed_"]', '#pmadv'],
        adBlockFinland: ['.mainostila', fromB64('LnNwb25zb3JpdA=='), '.ylamainos', fromB64('YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd')],
        adBlockPersian: ['#navbar_notice_50', '.kadr', 'TABLE[width="140px"]', '#divAgahi', fromB64('YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd')],
        adBlockWarningRemoval: ['#adblock-honeypot', '.adblocker-root', '.wp_adblock_detect', fromB64('LmhlYWRlci1ibG9ja2VkLWFk'), fromB64('I2FkX2Jsb2NrZXI=')],
        adGuardAnnoyances: ['.hs-sosyal', '#cookieconsentdiv', 'div[class^="app_gdpr"]', '.as-oil', '[data-cypress="soft-push-notification-modal"]'],
        adGuardBase: ['.BetterJsPopOverlay', fromB64('I2FkXzMwMFgyNTA='), fromB64('I2Jhbm5lcmZsb2F0MjI='), fromB64('I2NhbXBhaWduLWJhbm5lcg=='), fromB64('I0FkLUNvbnRlbnQ=')],
        adGuardChinese: [fromB64('LlppX2FkX2FfSA=='), fromB64('YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd'), '#widget-quan', fromB64('YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd'), fromB64('YVtocmVmKj0iLjE5NTZobC5jb20vIl0=')],
        adGuardFrench: ['#pavePub', fromB64('LmFkLWRlc2t0b3AtcmVjdGFuZ2xl'), '.mobile_adhesion', '.widgetadv', fromB64('LmFkc19iYW4=')],
        adGuardGerman: ['aside[data-portal-id="leaderboard"]'],
        adGuardJapanese: ['#kauli_yad_1', fromB64('YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0='), fromB64('Ll9wb3BJbl9pbmZpbml0ZV9hZA=='), fromB64('LmFkZ29vZ2xl'), fromB64('Ll9faXNib29zdFJldHVybkFk')],
        adGuardMobile: [fromB64('YW1wLWF1dG8tYWRz'), fromB64('LmFtcF9hZA=='), 'amp-embed[type="24smi"]', '#mgid_iframe1', fromB64('I2FkX2ludmlld19hcmVh')],
        adGuardRussian: [fromB64('YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0='), fromB64('LnJlY2xhbWE='), 'div[id^="smi2adblock"]', fromB64('ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd'), '#psyduckpockeball'],
        adGuardSocial: [fromB64('YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0='), fromB64('YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0='), '.etsy-tweet', '#inlineShare', '.popup-social'],
        adGuardSpanishPortuguese: ['#barraPublicidade', '#Publicidade', '#publiEspecial', '#queTooltip', '.cnt-publi'],
        adGuardTrackingProtection: ['#qoo-counter', fromB64('YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=='), fromB64('YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0='), fromB64('YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=='), '#top100counter'],
        adGuardTurkish: ['#backkapat', fromB64('I3Jla2xhbWk='), fromB64('YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0='), fromB64('YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd'), fromB64('YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ==')],
        bulgarian: [fromB64('dGQjZnJlZW5ldF90YWJsZV9hZHM='), '#ea_intext_div', '.lapni-pop-over', '#xenium_hot_offers'],
        easyList: ['.yb-floorad', fromB64('LndpZGdldF9wb19hZHNfd2lkZ2V0'), fromB64('LnRyYWZmaWNqdW5reS1hZA=='), '.textad_headline', fromB64('LnNwb25zb3JlZC10ZXh0LWxpbmtz')],
        easyListChina: [fromB64('LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=='), fromB64('LmZyb250cGFnZUFkdk0='), '#taotaole', '#aafoot.top_box', '.cfa_popup'],
        easyListCookie: ['.ezmob-footer', '.cc-CookieWarning', '[data-cookie-number]', fromB64('LmF3LWNvb2tpZS1iYW5uZXI='), '.sygnal24-gdpr-modal-wrap'],
        easyListCzechSlovak: ['#onlajny-stickers', fromB64('I3Jla2xhbW5pLWJveA=='), fromB64('LnJla2xhbWEtbWVnYWJvYXJk'), '.sklik', fromB64('W2lkXj0ic2tsaWtSZWtsYW1hIl0=')],
        easyListDutch: [fromB64('I2FkdmVydGVudGll'), fromB64('I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=='), '.adstekst', fromB64('YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0='), '#semilo-lrectangle'],
        easyListGermany: ['#SSpotIMPopSlider', fromB64('LnNwb25zb3JsaW5rZ3J1ZW4='), fromB64('I3dlcmJ1bmdza3k='), fromB64('I3Jla2xhbWUtcmVjaHRzLW1pdHRl'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0=')],
        easyListItaly: [fromB64('LmJveF9hZHZfYW5udW5jaQ=='), '.sb-box-pubbliredazionale', fromB64('YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ==')],
        easyListLithuania: [fromB64('LnJla2xhbW9zX3RhcnBhcw=='), fromB64('LnJla2xhbW9zX251b3JvZG9z'), fromB64('aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd'), fromB64('aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd'), fromB64('aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd')],
        estonian: [fromB64('QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ==')],
        fanboyAnnoyances: ['#ac-lre-player', '.navigate-to-top', '#subscribe_popup', '.newsletter_holder', '#back-top'],
        fanboyAntiFacebook: ['.util-bar-module-firefly-visible'],
        fanboyEnhancedTrackers: ['.open.pushModal', '#issuem-leaky-paywall-articles-zero-remaining-nag', '#sovrn_container', 'div[class$="-hide"][zoompage-fontsize][style="display: block;"]', '.BlockNag__Card'],
        fanboySocial: ['#FollowUs', '#meteored_share', '#social_follow', '.article-sharer', '.community__social-desc'],
        frellwitSwedish: [fromB64('YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=='), fromB64('YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=='), 'article.category-samarbete', fromB64('ZGl2LmhvbGlkQWRz'), 'ul.adsmodern'],
        greekAdBlock: [fromB64('QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd'), fromB64('QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=='), fromB64('QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd'), 'DIV.agores300', 'TABLE.advright'],
        hungarian: ['#cemp_doboz', '.optimonk-iframe-container', fromB64('LmFkX19tYWlu'), fromB64('W2NsYXNzKj0iR29vZ2xlQWRzIl0='), '#hirdetesek_box'],
        iDontCareAboutCookies: ['.alert-info[data-block-track*="CookieNotice"]', '.ModuleTemplateCookieIndicator', '.o--cookies--container', '#cookies-policy-sticky', '#stickyCookieBar'],
        icelandicAbp: [fromB64('QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ==')],
        latvian: [
          fromB64('YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0O' + 'iA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0='),
          fromB64('YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6I' + 'DMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ==')
        ],
        listKr: [fromB64('YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0='), fromB64('I2xpdmVyZUFkV3JhcHBlcg=='), fromB64('YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=='), fromB64('aW5zLmZhc3R2aWV3LWFk'), '.revenue_unit_item.dable'],
        listeAr: [fromB64('LmdlbWluaUxCMUFk'), '.right-and-left-sponsers', fromB64('YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=='), fromB64('YVtocmVmKj0iYm9vcmFxLm9yZyJd'), fromB64('YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd')],
        listeFr: [fromB64('YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=='), fromB64('I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=='), fromB64('YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0='), '.site-pub-interstitiel', 'div[id^="crt-"][data-criteo-id]'],
        officialPolish: ['#ceneo-placeholder-ceneo-12', fromB64('W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd'), fromB64('YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=='), fromB64('YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=='), fromB64('ZGl2I3NrYXBpZWNfYWQ=')],
        ro: [fromB64('YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd'), fromB64('YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0='), fromB64('YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd'), 'a[href^="/url/"]'],
        ruAd: [fromB64('YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd'), fromB64('YVtocmVmKj0iLy91dGltZy5ydS8iXQ=='), fromB64('YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0='), '#pgeldiz', '.yandex-rtb-block'],
        thaiAds: ['a[href*=macau-uta-popup]', fromB64('I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=='), fromB64('LmFkczMwMHM='), '.bumq', '.img-kosana'],
        webAnnoyancesUltralist: ['#mod-social-share-2', '#social-tools', fromB64('LmN0cGwtZnVsbGJhbm5lcg=='), '.zergnet-recommend', '.yt.btn-link.btn-md.btn']
      };
    })()
  |}]

let force_show (element : js) : unit =
  let style = get element "style" in
  ignore
    (invoke3 style "setProperty" (of_string "visibility") (of_string "hidden")
       (of_string "important"));
  ignore
    (invoke3 style "setProperty" (of_string "display") (of_string "block")
       (of_string "important"))

let get_blocked_selectors (selectors : js) : js =
  let d = document () in
  let root = invoke1 d "createElement" (of_string "div") in
  let elements = array_ () in
  let blocked = object_ () in
  force_show root;
  let rec create i =
    if i >= array_length selectors then ()
    else
      let selector = to_string (get_i selectors i) in
      let element = Dom.selector_to_element selector in
      if to_string (get element "tagName") = "DIALOG" then
        ignore (invoke0 element "show");
      let holder = invoke1 d "createElement" (of_string "div") in
      force_show holder;
      ignore (invoke1 holder "appendChild" element);
      ignore (invoke1 root "appendChild" holder);
      ignore (push elements element);
      create (i + 1)
  in
  create 0;
  let rec wait_body () =
    if truthy (get d "body") then promise_resolve undefined
    else promise_then (wait 50.) (fun _ -> wait_body ())
  in
  promise_then (wait_body ()) (fun _ ->
      ignore (invoke1 (get d "body") "appendChild" root);
      let check () =
        let rec loop i =
          if i >= array_length selectors then ()
          else (
            if not (truthy (get (get_i elements i) "offsetParent")) then
              set blocked (to_string (get_i selectors i)) (of_bool true);
            loop (i + 1))
        in
        loop 0
      in
      let wrap : (unit -> unit) -> js -> js =
        [%mel.raw
          {|
          function(check, root) {
            try { check(); }
            finally {
              var parent = root.parentNode;
              if (parent) parent.removeChild(root);
            }
          }
        |}]
      in
      ignore (wrap check root);
      blocked)

let print_debug (filters : js) (blocked : js) : unit =
  let f : js -> js -> unit =
    [%mel.raw
      {|
      function(filters, blocked) {
        var names = Object.keys(filters);
        var message = "DOM blockers debug:\n```";
        for (var i = 0; i < names.length; i++) {
          var filterName = names[i];
          var selectors = filters[filterName];
          message += "\n" + filterName + ":";
          for (var j = 0; j < selectors.length; j++) {
            var selector = selectors[j];
            var mark = blocked[selector] ? "🚫" : "➡️";
            message += "\n  " + mark + " " + selector;
          }
        }
        console.log(message + "\n```");
      }
    |}]
  in
  f filters blocked

let is_applicable () = Browser.is_webkit () || Browser.is_android ()

let get_dom_blockers (options : js) : js =
  if not (is_applicable ()) then promise_resolve undefined
  else
    let filters = get_filters () in
    let filter_names = object_keys filters in
    let all = array_ () in
    let rec concat i =
      if i >= array_length filter_names then ()
      else
        let name = to_string (get_i filter_names i) in
        let selectors = get filters name in
        let rec add j =
          if j >= array_length selectors then ()
          else (
            ignore (push all (get_i selectors j));
            add (j + 1))
        in
        add 0;
        concat (i + 1)
    in
    concat 0;
    promise_then (get_blocked_selectors all) (fun blocked ->
        let debug = if is_undefined options then undefined else get options "debug" in
        if truthy debug then print_debug filters blocked;
        let active = array_ () in
        let rec pick i =
          if i >= array_length filter_names then ()
          else
            let filter_name = to_string (get_i filter_names i) in
            let selectors = get filters filter_name in
            let rec count j n =
              if j >= array_length selectors then n
              else
                let sel = to_string (get_i selectors j) in
                count (j + 1) (n + if truthy (get blocked sel) then 1 else 0)
            in
            let blocked_count = count 0 0 in
            if float_of_int blocked_count > float_of_int (array_length selectors) *. 0.6
            then ignore (push active (of_string filter_name));
            pick (i + 1)
        in
        pick 0;
        ignore (array_sort active);
        active)
