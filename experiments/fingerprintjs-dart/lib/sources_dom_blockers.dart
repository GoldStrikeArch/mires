library;

import 'dart:js_interop';

import 'browser.dart';
import 'data.dart';
import 'dom.dart';
import 'fp_async.dart';
import 'host.dart';

Map<String, List<String>> getFilters() {
  String b64(String encoded) => jsAtob(encoded);
  return {
    'abpIndo': [
      '#Iklan-Melayang',
      '#Kolom-Iklan-728',
      '#SidebarIklan-wrapper',
      '[title="ALIENBOLA" i]',
      b64('I0JveC1CYW5uZXItYWRz'),
    ],
    'abpvn': ['.quangcao', '#mobileCatfish', b64('LmNsb3NlLWFkcw=='), '[id^="bn_bottom_fixed_"]', '#pmadv'],
    'adBlockFinland': [
      '.mainostila',
      b64('LnNwb25zb3JpdA=='),
      '.ylamainos',
      b64('YVtocmVmKj0iL2NsaWNrdGhyZ2guYXNwPyJd'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9hcHAucmVhZHBlYWsuY29tL2FkcyJd'),
    ],
    'adBlockPersian': [
      '#navbar_notice_50',
      '.kadr',
      'TABLE[width="140px"]',
      '#divAgahi',
      b64('YVtocmVmXj0iaHR0cDovL2cxLnYuZndtcm0ubmV0L2FkLyJd'),
    ],
    'adBlockWarningRemoval': [
      '#adblock-honeypot',
      '.adblocker-root',
      '.wp_adblock_detect',
      b64('LmhlYWRlci1ibG9ja2VkLWFk'),
      b64('I2FkX2Jsb2NrZXI='),
    ],
    'adGuardAnnoyances': [
      '.hs-sosyal',
      '#cookieconsentdiv',
      'div[class^="app_gdpr"]',
      '.as-oil',
      '[data-cypress="soft-push-notification-modal"]',
    ],
    'adGuardBase': [
      '.BetterJsPopOverlay',
      b64('I2FkXzMwMFgyNTA='),
      b64('I2Jhbm5lcmZsb2F0MjI='),
      b64('I2NhbXBhaWduLWJhbm5lcg=='),
      b64('I0FkLUNvbnRlbnQ='),
    ],
    'adGuardChinese': [
      b64('LlppX2FkX2FfSA=='),
      b64('YVtocmVmKj0iLmh0aGJldDM0LmNvbSJd'),
      '#widget-quan',
      b64('YVtocmVmKj0iLzg0OTkyMDIwLnh5eiJd'),
      b64('YVtocmVmKj0iLjE5NTZobC5jb20vIl0='),
    ],
    'adGuardFrench': [
      '#pavePub',
      b64('LmFkLWRlc2t0b3AtcmVjdGFuZ2xl'),
      '.mobile_adhesion',
      '.widgetadv',
      b64('LmFkc19iYW4='),
    ],
    'adGuardGerman': ['aside[data-portal-id="leaderboard"]'],
    'adGuardJapanese': [
      '#kauli_yad_1',
      b64('YVtocmVmXj0iaHR0cDovL2FkMi50cmFmZmljZ2F0ZS5uZXQvIl0='),
      b64('Ll9wb3BJbl9pbmZpbml0ZV9hZA=='),
      b64('LmFkZ29vZ2xl'),
      b64('Ll9faXNib29zdFJldHVybkFk'),
    ],
    'adGuardMobile': [
      b64('YW1wLWF1dG8tYWRz'),
      b64('LmFtcF9hZA=='),
      'amp-embed[type="24smi"]',
      '#mgid_iframe1',
      b64('I2FkX2ludmlld19hcmVh'),
    ],
    'adGuardRussian': [
      b64('YVtocmVmXj0iaHR0cHM6Ly9hZC5sZXRtZWFkcy5jb20vIl0='),
      b64('LnJlY2xhbWE='),
      'div[id^="smi2adblock"]',
      b64('ZGl2W2lkXj0iQWRGb3hfYmFubmVyXyJd'),
      '#psyduckpockeball',
    ],
    'adGuardSocial': [
      b64('YVtocmVmXj0iLy93d3cuc3R1bWJsZXVwb24uY29tL3N1Ym1pdD91cmw9Il0='),
      b64('YVtocmVmXj0iLy90ZWxlZ3JhbS5tZS9zaGFyZS91cmw/Il0='),
      '.etsy-tweet',
      '#inlineShare',
      '.popup-social',
    ],
    'adGuardSpanishPortuguese': ['#barraPublicidade', '#Publicidade', '#publiEspecial', '#queTooltip', '.cnt-publi'],
    'adGuardTrackingProtection': [
      '#qoo-counter',
      b64('YVtocmVmXj0iaHR0cDovL2NsaWNrLmhvdGxvZy5ydS8iXQ=='),
      b64('YVtocmVmXj0iaHR0cDovL2hpdGNvdW50ZXIucnUvdG9wL3N0YXQucGhwIl0='),
      b64('YVtocmVmXj0iaHR0cDovL3RvcC5tYWlsLnJ1L2p1bXAiXQ=='),
      '#top100counter',
    ],
    'adGuardTurkish': [
      '#backkapat',
      b64('I3Jla2xhbWk='),
      b64('YVtocmVmXj0iaHR0cDovL2Fkc2Vydi5vbnRlay5jb20udHIvIl0='),
      b64('YVtocmVmXj0iaHR0cDovL2l6bGVuemkuY29tL2NhbXBhaWduLyJd'),
      b64('YVtocmVmXj0iaHR0cDovL3d3dy5pbnN0YWxsYWRzLm5ldC8iXQ=='),
    ],
    'bulgarian': [b64('dGQjZnJlZW5ldF90YWJsZV9hZHM='), '#ea_intext_div', '.lapni-pop-over', '#xenium_hot_offers'],
    'easyList': [
      '.yb-floorad',
      b64('LndpZGdldF9wb19hZHNfd2lkZ2V0'),
      b64('LnRyYWZmaWNqdW5reS1hZA=='),
      '.textad_headline',
      b64('LnNwb25zb3JlZC10ZXh0LWxpbmtz'),
    ],
    'easyListChina': [
      b64('LmFwcGd1aWRlLXdyYXBbb25jbGljayo9ImJjZWJvcy5jb20iXQ=='),
      b64('LmZyb250cGFnZUFkdk0='),
      '#taotaole',
      '#aafoot.top_box',
      '.cfa_popup',
    ],
    'easyListCookie': [
      '.ezmob-footer',
      '.cc-CookieWarning',
      '[data-cookie-number]',
      b64('LmF3LWNvb2tpZS1iYW5uZXI='),
      '.sygnal24-gdpr-modal-wrap',
    ],
    'easyListCzechSlovak': [
      '#onlajny-stickers',
      b64('I3Jla2xhbW5pLWJveA=='),
      b64('LnJla2xhbWEtbWVnYWJvYXJk'),
      '.sklik',
      b64('W2lkXj0ic2tsaWtSZWtsYW1hIl0='),
    ],
    'easyListDutch': [
      b64('I2FkdmVydGVudGll'),
      b64('I3ZpcEFkbWFya3RCYW5uZXJCbG9jaw=='),
      '.adstekst',
      b64('YVtocmVmXj0iaHR0cHM6Ly94bHR1YmUubmwvY2xpY2svIl0='),
      '#semilo-lrectangle',
    ],
    'easyListGermany': [
      '#SSpotIMPopSlider',
      b64('LnNwb25zb3JsaW5rZ3J1ZW4='),
      b64('I3dlcmJ1bmdza3k='),
      b64('I3Jla2xhbWUtcmVjaHRzLW1pdHRl'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9iZDc0Mi5jb20vIl0='),
    ],
    'easyListItaly': [
      b64('LmJveF9hZHZfYW5udW5jaQ=='),
      '.sb-box-pubbliredazionale',
      b64('YVtocmVmXj0iaHR0cDovL2FmZmlsaWF6aW9uaWFkcy5zbmFpLml0LyJd'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9hZHNlcnZlci5odG1sLml0LyJd'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9hZmZpbGlhemlvbmlhZHMuc25haS5pdC8iXQ=='),
    ],
    'easyListLithuania': [
      b64('LnJla2xhbW9zX3RhcnBhcw=='),
      b64('LnJla2xhbW9zX251b3JvZG9z'),
      b64('aW1nW2FsdD0iUmVrbGFtaW5pcyBza3lkZWxpcyJd'),
      b64('aW1nW2FsdD0iRGVkaWt1b3RpLmx0IHNlcnZlcmlhaSJd'),
      b64('aW1nW2FsdD0iSG9zdGluZ2FzIFNlcnZlcmlhaS5sdCJd'),
    ],
    'estonian': [b64('QVtocmVmKj0iaHR0cDovL3BheTRyZXN1bHRzMjQuZXUiXQ==')],
    'fanboyAnnoyances': ['#ac-lre-player', '.navigate-to-top', '#subscribe_popup', '.newsletter_holder', '#back-top'],
    'fanboyAntiFacebook': ['.util-bar-module-firefly-visible'],
    'fanboyEnhancedTrackers': [
      '.open.pushModal',
      '#issuem-leaky-paywall-articles-zero-remaining-nag',
      '#sovrn_container',
      'div[class\$="-hide"][zoompage-fontsize][style="display: block;"]',
      '.BlockNag__Card',
    ],
    'fanboySocial': ['#FollowUs', '#meteored_share', '#social_follow', '.article-sharer', '.community__social-desc'],
    'frellwitSwedish': [
      b64('YVtocmVmKj0iY2FzaW5vcHJvLnNlIl1bdGFyZ2V0PSJfYmxhbmsiXQ=='),
      b64('YVtocmVmKj0iZG9rdG9yLXNlLm9uZWxpbmsubWUiXQ=='),
      'article.category-samarbete',
      b64('ZGl2LmhvbGlkQWRz'),
      'ul.adsmodern',
    ],
    'greekAdBlock': [
      b64('QVtocmVmKj0iYWRtYW4ub3RlbmV0LmdyL2NsaWNrPyJd'),
      b64('QVtocmVmKj0iaHR0cDovL2F4aWFiYW5uZXJzLmV4b2R1cy5nci8iXQ=='),
      b64('QVtocmVmKj0iaHR0cDovL2ludGVyYWN0aXZlLmZvcnRobmV0LmdyL2NsaWNrPyJd'),
      'DIV.agores300',
      'TABLE.advright',
    ],
    'hungarian': [
      '#cemp_doboz',
      '.optimonk-iframe-container',
      b64('LmFkX19tYWlu'),
      b64('W2NsYXNzKj0iR29vZ2xlQWRzIl0='),
      '#hirdetesek_box',
    ],
    'iDontCareAboutCookies': [
      '.alert-info[data-block-track*="CookieNotice"]',
      '.ModuleTemplateCookieIndicator',
      '.o--cookies--container',
      '#cookies-policy-sticky',
      '#stickyCookieBar',
    ],
    'icelandicAbp': [b64('QVtocmVmXj0iL2ZyYW1ld29yay9yZXNvdXJjZXMvZm9ybXMvYWRzLmFzcHgiXQ==')],
    'latvian': [
      b64(
        'YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiAxMjBweDsgaGVpZ2h0O'
        'iA0MHB4OyBvdmVyZmxvdzogaGlkZGVuOyBwb3NpdGlvbjogcmVsYXRpdmU7Il0=',
      ),
      b64(
        'YVtocmVmPSJodHRwOi8vd3d3LnNhbGlkemluaS5sdi8iXVtzdHlsZT0iZGlzcGxheTogYmxvY2s7IHdpZHRoOiA4OHB4OyBoZWlnaHQ6I'
        'DMxcHg7IG92ZXJmbG93OiBoaWRkZW47IHBvc2l0aW9uOiByZWxhdGl2ZTsiXQ==',
      ),
    ],
    'listKr': [
      b64('YVtocmVmKj0iLy9hZC5wbGFuYnBsdXMuY28ua3IvIl0='),
      b64('I2xpdmVyZUFkV3JhcHBlcg=='),
      b64('YVtocmVmKj0iLy9hZHYuaW1hZHJlcC5jby5rci8iXQ=='),
      b64('aW5zLmZhc3R2aWV3LWFk'),
      '.revenue_unit_item.dable',
    ],
    'listeAr': [
      b64('LmdlbWluaUxCMUFk'),
      '.right-and-left-sponsers',
      b64('YVtocmVmKj0iLmFmbGFtLmluZm8iXQ=='),
      b64('YVtocmVmKj0iYm9vcmFxLm9yZyJd'),
      b64('YVtocmVmKj0iZHViaXp6bGUuY29tL2FyLz91dG1fc291cmNlPSJd'),
    ],
    'listeFr': [
      b64('YVtocmVmXj0iaHR0cDovL3Byb21vLnZhZG9yLmNvbS8iXQ=='),
      b64('I2FkY29udGFpbmVyX3JlY2hlcmNoZQ=='),
      b64('YVtocmVmKj0id2Vib3JhbWEuZnIvZmNnaS1iaW4vIl0='),
      '.site-pub-interstitiel',
      'div[id^="crt-"][data-criteo-id]',
    ],
    'officialPolish': [
      '#ceneo-placeholder-ceneo-12',
      b64('W2hyZWZePSJodHRwczovL2FmZi5zZW5kaHViLnBsLyJd'),
      b64('YVtocmVmXj0iaHR0cDovL2Fkdm1hbmFnZXIudGVjaGZ1bi5wbC9yZWRpcmVjdC8iXQ=='),
      b64('YVtocmVmXj0iaHR0cDovL3d3dy50cml6ZXIucGwvP3V0bV9zb3VyY2UiXQ=='),
      b64('ZGl2I3NrYXBpZWNfYWQ='),
    ],
    'ro': [
      b64('YVtocmVmXj0iLy9hZmZ0cmsuYWx0ZXgucm8vQ291bnRlci9DbGljayJd'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9ibGFja2ZyaWRheXNhbGVzLnJvL3Ryay9zaG9wLyJd'),
      b64('YVtocmVmXj0iaHR0cHM6Ly9ldmVudC4ycGVyZm9ybWFudC5jb20vZXZlbnRzL2NsaWNrIl0='),
      b64('YVtocmVmXj0iaHR0cHM6Ly9sLnByb2ZpdHNoYXJlLnJvLyJd'),
      'a[href^="/url/"]',
    ],
    'ruAd': [
      b64('YVtocmVmKj0iLy9mZWJyYXJlLnJ1LyJd'),
      b64('YVtocmVmKj0iLy91dGltZy5ydS8iXQ=='),
      b64('YVtocmVmKj0iOi8vY2hpa2lkaWtpLnJ1Il0='),
      '#pgeldiz',
      '.yandex-rtb-block',
    ],
    'thaiAds': [
      'a[href*=macau-uta-popup]',
      b64('I2Fkcy1nb29nbGUtbWlkZGxlX3JlY3RhbmdsZS1ncm91cA=='),
      b64('LmFkczMwMHM='),
      '.bumq',
      '.img-kosana',
    ],
    'webAnnoyancesUltralist': [
      '#mod-social-share-2',
      '#social-tools',
      b64('LmN0cGwtZnVsbGJhbm5lcg=='),
      '.zergnet-recommend',
      '.yt.btn-link.btn-md.btn',
    ],
  };
}

JSAny? getDomBlockers([JSAny? options]) {
  return futureToPromise(() async {
    if (!isApplicable()) return jsUndefined;
    final filters = getFilters();
    final filterNames = filters.keys.toList();
    final allSelectors = <String>[];
    for (final name in filterNames) {
      allSelectors.addAll(filters[name]!);
    }
    final blockedSelectors = await getBlockedSelectors(allSelectors);
    if (jsTruthy(getp(options, 'debug'))) {
      _printDebug(filters, blockedSelectors);
    }
    final active = <String>[];
    for (final name in filterNames) {
      final selectors = filters[name]!;
      var blockedCount = 0;
      for (final selector in selectors) {
        if (blockedSelectors[selector] == true) blockedCount++;
      }
      if (blockedCount > selectors.length * 0.6) active.add(name);
    }
    active.sort();
    return fromDart(active);
  }());
}

bool isApplicable() => isWebKit() || isAndroid();

Future<Map<String, bool>> getBlockedSelectors(List<String> selectors) async {
  final d = jsDocument;
  final root = method(d, 'createElement', ['div'.toJS]);
  final elements = <JSAny?>[];
  final blocked = <String, bool>{};
  _forceShow(root);
  for (final selector in selectors) {
    final element = selectorToElement(selector);
    if (jsString(getp(element, 'tagName')) == 'DIALOG') {
      method(element, 'show');
    }
    final holder = method(d, 'createElement', ['div'.toJS]);
    _forceShow(holder);
    method(holder, 'appendChild', [element]);
    method(root, 'appendChild', [holder]);
    elements.add(element);
  }
  while (!jsTruthy(getp(d, 'body'))) {
    await (wait(50) as JSPromise<JSAny?>).toDart;
  }
  method(getp(d, 'body'), 'appendChild', [root]);
  try {
    for (var i = 0; i < selectors.length; i++) {
      if (!jsTruthy(getp(elements[i], 'offsetParent'))) {
        blocked[selectors[i]] = true;
      }
    }
  } finally {
    final parent = getp(root, 'parentNode');
    if (parent.isDefinedAndNotNull) {
      method(parent, 'removeChild', [root]);
    }
  }
  return blocked;
}

void _forceShow(JSAny? element) {
  final style = getp(element, 'style');
  method(style, 'setProperty', ['visibility'.toJS, 'hidden'.toJS, 'important'.toJS]);
  method(style, 'setProperty', ['display'.toJS, 'block'.toJS, 'important'.toJS]);
}

void _printDebug(Map<String, List<String>> filters, Map<String, bool> blocked) {
  final message = StringBuffer('DOM blockers debug:\n```');
  for (final name in filters.keys) {
    message.write('\n$name:');
    for (final selector in filters[name]!) {
      message.write('\n  ${blocked[selector] == true ? '🚫' : '➡️'} $selector');
    }
  }
  logDebug('${message}\n```');
}
