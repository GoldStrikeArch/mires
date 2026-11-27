use itertools::{Either, EitherOrBoth, FoldWhile, Itertools};

#[test]
fn windows_tuples_and_combinatorics() {
    let wins: Vec<(i32, i32, i32)> = (1..=5).tuple_windows().collect();
    assert_eq!(wins, vec![(1, 2, 3), (2, 3, 4), (3, 4, 5)]);

    let pairs: Vec<(i32, i32)> = (1..=6).tuples().collect();
    assert_eq!(pairs, vec![(1, 2), (3, 4), (5, 6)]);

    let combos: Vec<Vec<i32>> = (1..=4).combinations(2).collect();
    assert_eq!(
        combos,
        vec![
            vec![1, 2],
            vec![1, 3],
            vec![1, 4],
            vec![2, 3],
            vec![2, 4],
            vec![3, 4]
        ]
    );

    let cart: Vec<(i32, char)> = [1, 2]
        .into_iter()
        .cartesian_product(['a', 'b', 'c'])
        .collect();
    assert_eq!(
        cart,
        vec![(1, 'a'), (1, 'b'), (1, 'c'), (2, 'a'), (2, 'b'), (2, 'c')]
    );

    let sorted: Vec<i32> = [5, 2, 9, 1, 5, 6].into_iter().sorted().collect();
    assert_eq!(sorted, vec![1, 2, 5, 5, 6, 9]);
}

#[test]
fn merge_dedup_chunk_by_and_unique() {
    let merged: Vec<i32> = [1, 4, 7].into_iter().merge([2, 3, 8]).collect();
    assert_eq!(merged, vec![1, 2, 3, 4, 7, 8]);

    let kmerged: Vec<i32> = vec![vec![1, 5], vec![2, 6], vec![0, 3, 4]]
        .into_iter()
        .kmerge()
        .collect();
    assert_eq!(kmerged, vec![0, 1, 2, 3, 4, 5, 6]);

    let rle: Vec<(usize, char)> = "aaabbcdddd".chars().dedup_with_count().collect();
    assert_eq!(rle, vec![(3, 'a'), (2, 'b'), (1, 'c'), (4, 'd')]);

    let grouped: Vec<(bool, Vec<i32>)> = [1, 3, 2, 4, 5]
        .into_iter()
        .chunk_by(|x| x % 2 == 0)
        .into_iter()
        .map(|(k, g)| (k, g.collect()))
        .collect();
    assert_eq!(
        grouped,
        vec![(false, vec![1, 3]), (true, vec![2, 4]), (false, vec![5])]
    );

    let uniq: Vec<i32> = [3, 1, 2, 1, 3, 4, 2].into_iter().unique().collect();
    assert_eq!(uniq, vec![3, 1, 2, 4]);
}

#[test]
fn join_zip_longest_partition_and_fold_while() {
    assert_eq!(["x", "y", "z"].into_iter().join("::"), "x::y::z");

    let zl: Vec<EitherOrBoth<i32, char>> =
        [1, 2].into_iter().zip_longest(['a', 'b', 'c']).collect();
    assert_eq!(
        zl,
        vec![
            EitherOrBoth::Both(1, 'a'),
            EitherOrBoth::Both(2, 'b'),
            EitherOrBoth::Right('c'),
        ]
    );

    let (evens, odds): (Vec<i32>, Vec<i32>) = (1..=6).partition_map(|x| {
        if x % 2 == 0 {
            Either::Left(x)
        } else {
            Either::Right(x)
        }
    });
    assert_eq!(evens, vec![2, 4, 6]);
    assert_eq!(odds, vec![1, 3, 5]);

    let sum = [2, 3, 4, 10, 5]
        .into_iter()
        .fold_while(0, |acc, x| {
            if acc + x <= 10 {
                FoldWhile::Continue(acc + x)
            } else {
                FoldWhile::Done(acc)
            }
        })
        .into_inner();
    assert_eq!(sum, 9);
}
