open Host

let is_grease_brand (brand : string) : bool = re_test (new_re "not" "i") brand

let get_user_agent_data (_ : js) : js =
  let ua = get (navigator ()) "userAgentData" in
  if not (truthy ua) then promise_resolve undefined
  else
    let raw_brands = get ua "brands" in
    let filtered = array_ () in
    let rec filter i =
      if i >= array_length raw_brands then ()
      else
        let item = get_i raw_brands i in
        let brand = get item "brand" in
        if not (is_grease_brand (to_string brand)) then ignore (push filtered brand);
        filter (i + 1)
    in
    filter 0;
    let brands =
      if array_length filtered > 1 then
        let without = array_ () in
        let rec drop i =
          if i >= array_length filtered then ()
          else
            let brand = get_i filtered i in
            if to_string brand <> "Chromium" then ignore (push without brand);
            drop (i + 1)
        in
        drop 0;
        without
      else filtered
    in
    let result = object_ () in
    set result "brands" brands;
    set result "mobile" (get ua "mobile");
    set result "platform" (get ua "platform");
    let high = get ua "getHighEntropyValues" in
    if not (truthy high) then promise_resolve result
    else
      let hints = array_ () in
      ignore (push hints (of_string "architecture"));
      ignore (push hints (of_string "bitness"));
      ignore (push hints (of_string "model"));
      ignore (push hints (of_string "platformVersion"));
      let p = invoke1 ua "getHighEntropyValues" hints in
      promise_then2 p
        (fun high_entropy ->
          set result "architecture" (get high_entropy "architecture");
          set result "bitness" (get high_entropy "bitness");
          set result "model" (get high_entropy "model");
          set result "platformVersion" (get high_entropy "platformVersion");
          result)
        (fun error ->
          if is_dom_exception error && to_string (get error "name") = "NotAllowedError"
          then (
            set result "highEntropyStatus" (of_string "not_allowed");
            result)
          else
            let throw : js -> js =
              [%mel.raw {| function(e) { throw e; } |}]
            in
            throw error)
