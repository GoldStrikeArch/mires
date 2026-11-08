package fp

import scala.scalajs.js

object UserAgentData {
  private def isGreaseBrand(brand: String): Boolean =
    new js.RegExp("not", "i").test(brand)

  def getUserAgentData(_options: js.Any): js.Promise[js.Any] = {
    val uaData = Host.navigator.selectDynamic("userAgentData")
    if (Host.isUndef(uaData.asInstanceOf[js.Any])) return js.Promise.resolve[js.Any](js.undefined)

    val brandsRaw = uaData.brands.asInstanceOf[js.Array[js.Dynamic]]
    val filtered = brandsRaw.filter(item => !isGreaseBrand(Host.toJsString(item.brand.asInstanceOf[js.Any])))
      .map(item => Host.toJsString(item.brand.asInstanceOf[js.Any]))
    val brands =
      if (filtered.length > 1) filtered.filter(b => b != "Chromium")
      else filtered

    val result = js.Dynamic.literal(
      brands = brands,
      mobile = uaData.mobile,
      platform = uaData.platform,
    )

    val getHighEntropyValues = uaData.selectDynamic("getHighEntropyValues")
    if (!Host.isJsFunction(getHighEntropyValues.asInstanceOf[js.Any]))
      return js.Promise.resolve[js.Any](result)

    uaData
      .getHighEntropyValues(js.Array("architecture", "bitness", "model", "platformVersion"))
      .`asInstanceOf`[js.Promise[js.Dynamic]]
      .`then`[js.Any] { (highEntropy: js.Dynamic) =>
        result.architecture = highEntropy.architecture
        result.bitness = highEntropy.bitness
        result.model = highEntropy.model
        result.platformVersion = highEntropy.platformVersion
        result.asInstanceOf[js.Any]
      }
      .`catch` { (error: Any) =>
        val jsError = error.asInstanceOf[js.Any]
        if (
          Host.instanceofDomException(jsError) &&
          Host.toJsString(Host.dyn(jsError).name.asInstanceOf[js.Any]) == "NotAllowedError"
        ) {
          result.highEntropyStatus = "not_allowed"
          js.Promise.resolve[js.Any](result)
        } else js.Promise.reject(jsError)
      }
  }

  val source: js.Function1[js.Any, js.Promise[js.Any]] = getUserAgentData
}
