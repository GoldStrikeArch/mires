package main

import (
	"github.com/gopherjs/gopherjs/js"
	"sort"
	"strings"
)

func str(obj *js.Object, key string) string {
	v := obj.Get(key)
	if v == js.Undefined || v == nil {
		return ""
	}
	return v.String()
}
func number(obj *js.Object, key string) float64 {
	v := obj.Get(key)
	if v == js.Undefined {
		return 0
	}
	return v.Float()
}
func slash(s string, i int) bool {
	if i < 0 || i >= len(s) || s[i] != '/' {
		return false
	}
	k := i
	for k > 0 && s[k-1] == '\\' {
		k--
	}
	return (i-k)%2 == 0
}
func normalize(s string) string {
	if slash(s, len(s)-1) && !slash(s, len(s)-2) {
		s = s[:len(s)-1]
	}
	if !strings.HasPrefix(s, "/") {
		s = "/" + s
	}
	return s
}
func parentPath(s string) string {
	if len(s) < 2 {
		return ""
	}
	i := len(s) - 1
	for i > 1 && !slash(s, i) {
		i--
	}
	return s[:i]
}
func walk(nodes []*node, root int, post bool) []int {
	order := make([]int, 0, len(nodes))
	stack := []int{root}
	for len(stack) > 0 {
		v := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		order = append(order, v)
		kids := nodes[v].kids
		if post {
			stack = append(stack, kids...)
		} else {
			for j := len(kids) - 1; j >= 0; j-- {
				stack = append(stack, kids[j])
			}
		}
	}
	if post {
		for i, j := 0, len(order)-1; i < j; i, j = i+1, j-1 {
			order[i], order[j] = order[j], order[i]
		}
	}
	return order
}
func read(input *js.Object) ([]*node, int) {
	var nodes []*node
	mode := str(input, "source")
	if mode != "id" && mode != "path" {
		type item struct {
			d *js.Object
			p int
		}
		todo := []item{{input.Get("data"), -1}}
		for len(todo) > 0 {
			top := todo[len(todo)-1]
			todo = todo[:len(todo)-1]
			id := len(nodes)
			d, p := top.d, top.p
			nodes = append(nodes, &node{id: str(d, "id"), value: number(d, "value"), parent: p})
			if p >= 0 {
				nodes[p].kids = append(nodes[p].kids, id)
			}
			if c := d.Get("children"); c != js.Undefined && c != nil {
				for j := c.Length() - 1; j >= 0; j-- {
					todo = append(todo, item{c.Index(j), id})
				}
			}
		}
		return nodes, 0
	}
	rows := input.Get("rows")
	count := rows.Length()
	byID := map[string]int{}
	var parents []string
	for i := 0; i < count; i++ {
		row := rows.Index(i)
		id, parent := str(row, "id"), str(row, "parentId")
		if mode == "path" {
			id = normalize(id)
			parent = parentPath(id)
		}
		if id != "" {
			if _, ok := byID[id]; ok {
				byID[id] = -1
			} else {
				byID[id] = i
			}
		}
		nodes = append(nodes, &node{id: id, value: number(row, "value"), parent: -1})
		parents = append(parents, parent)
	}
	if mode == "path" {
		for i := 0; i < len(parents); i++ {
			id := parents[i]
			if _, ok := byID[id]; id != "" && !ok {
				byID[id] = len(nodes)
				nodes = append(nodes, &node{id: id, parent: -1})
				parents = append(parents, parentPath(id))
			}
		}
	}
	root := -1
	for i, parent := range parents {
		if parent == "" {
			if root >= 0 {
				panic("multiple roots")
			}
			root = i
		} else {
			p, ok := byID[parent]
			if !ok || p < 0 {
				panic("missing or ambiguous parent")
			}
			nodes[i].parent = p
			nodes[p].kids = append(nodes[p].kids, i)
		}
	}
	if root < 0 {
		panic("no root")
	}
	expected := len(nodes)
	if mode == "path" {
		for root >= count && len(nodes[root].kids) == 1 {
			root = nodes[root].kids[0]
			expected--
		}
	}
	nodes[root].parent = -1
	if len(walk(nodes, root, false)) != expected {
		panic("cycle")
	}
	return nodes, root
}
func run(input *js.Object) interface{} {
	nodes, root := read(input)
	for _, i := range walk(nodes, root, true) {
		n := nodes[i]
		if n.value == 0 {
			n.value = 0
		}
		for j := len(n.kids) - 1; j >= 0; j-- {
			c := nodes[n.kids[j]]
			n.value += c.value
			if c.height+1 > n.height {
				n.height = c.height + 1
			}
		}
		if input.Get("sort").Bool() {
			sort.SliceStable(n.kids, func(i, j int) bool { return nodes[n.kids[i]].value > nodes[n.kids[j]].value })
		}
	}
	pre := walk(nodes, root, false)
	post := walk(nodes, root, true)
	for _, i := range pre {
		n := nodes[i]
		if n.parent >= 0 {
			n.depth = nodes[n.parent].depth + 1
		}
	}
	op := str(input, "op")
	w, h, p := number(input, "width"), number(input, "height"), number(input, "padding")
	switch op {
	case "tree":
		tidy(nodes, pre, post, w, h, input.Get("nodeSize").Bool())
	case "cluster":
		cluster(nodes, pre, post, w, h, input.Get("nodeSize").Bool())
	case "pack":
		pack(nodes, pre, post, w, h, p)
	case "partition", "treemap":
		rectangles(nodes, pre, w, h, p, op == "partition", str(input, "tile"), input.Get("round").Bool())
	case "iterator":
		pre = []int{root}
		for j := 0; j < len(pre); j++ {
			pre = append(pre, nodes[pre[j]].kids...)
		}
	case "hierarchy", "stratify":
	default:
		panic("unknown operation")
	}
	out := make([]interface{}, 0, len(pre))
	for _, v := range pre {
		n := nodes[v]
		var parent interface{}
		if n.parent >= 0 {
			parent = nodes[n.parent].id
		}
		row := []interface{}{n.id, parent, n.depth, n.height, n.value}
		switch op {
		case "tree", "cluster":
			row = append(row, n.x, n.y)
		case "pack":
			row = append(row, n.x, n.y, n.r)
		case "partition", "treemap":
			row = append(row, n.x0, n.y0, n.x1, n.y1)
		}
		out = append(out, row)
	}
	return out
}
func main() { js.Module.Get("exports").Set("run", run) }
