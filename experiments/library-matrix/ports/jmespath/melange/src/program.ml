open Host
let run input=
  let data=get input "data"and qs=get input "queries"and result=array()in
  for i=0 to length qs-1 do let q=at qs i and out=obj()in
    (try let ast=Parser.compile(get q "expression")in
      let v=if strict(get q "op")(str "compile")then ast else Eval.eval ast data in
      set out "value" v
     with Parser.Error category->set out "error"(str category)|_->set out "error"(str "syntax"));
    push result out
  done;result
