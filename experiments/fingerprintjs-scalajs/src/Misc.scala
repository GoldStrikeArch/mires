package fp

import scala.scalajs.js
import scala.scalajs.js.JSConverters.*

object Misc {
  def errorToObject(error: js.Dynamic): js.Dynamic = {
    val stack = error.selectDynamic("stack")
    val result = js.Dynamic.literal(
      name = error.name,
      message = error.message,
      stack =
        if (Host.isUndefined(stack.asInstanceOf[js.Any]) || stack.asInstanceOf[js.Any] == null) js.undefined
        else Host.dyn(Host.toJsString(stack.asInstanceOf[js.Any])).split("\n"),
    )
    val extra = Host.objectKeys(error)
    var i = 0
    while (i < extra.length) {
      val key = extra(i)
      result.updateDynamic(key)(error.selectDynamic(key))
      i += 1
    }
    result
  }

  def isFunctionNative(func: js.Any): Boolean =
    new js.RegExp("""^function\s.*?\{\s*\[native code]\s*}$""").test(Host.toJsString(func))
}
