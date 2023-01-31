use rustc_ast::{ast, ptr::P, tokenstream::TokenStream, Pat, Ty};
use rustc_errors::PResult;
use rustc_expand::base::{self, DummyResult, ExtCtxt};
use rustc_span::{sym, Span};

pub fn expand(
    cx: &mut ExtCtxt<'_>,
    sp: Span,
    tts: TokenStream,
) -> Box<dyn base::MacResult + 'static> {
    let (ty, pat) = match parse_pat_ty(cx, tts) {
        Ok(parsed) => parsed,
        Err(err) => {
            err.emit();
            return DummyResult::any(sp);
        }
    };

    base::MacEager::ty(cx.ty(sp, ast::TyKind::Pat(ty, pat)))
}

fn parse_pat_ty<'a>(cx: &mut ExtCtxt<'a>, stream: TokenStream) -> PResult<'a, (P<Ty>, P<Pat>)> {
    let mut parser = cx.new_parser_from_tts(stream);

    let ty = parser.parse_ty()?;
    parser.eat_keyword(sym::is);
    let pat = parser.parse_pat_no_top_alt(None, None)?;

    Ok((ty, pat))
}
