open Host

let error_to_object (error : js) : js =
  let result = object_ () in
  set result "name" (get error "name");
  set result "message" (get error "message");
  let stack = get error "stack" in
  if is_undefined stack then set result "stack" undefined
  else set result "stack" (string_split (to_string stack) "\n");
  let keys = object_keys error in
  let rec loop i =
    if i >= array_length keys then ()
    else
      let key = get_i keys i in
      set result (to_string key) (get error (to_string key));
      loop (i + 1)
  in
  loop 0;
  result

let is_function_native (func : js) : bool =
  let re = new_re "^function\\s.*?\\{\\s*\\[native code]\\s*}$" "" in
  re_test re (string_value func)
