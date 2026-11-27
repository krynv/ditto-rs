//! macOS: Apple Vision.

use objc2::rc::autoreleasepool;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{AnyThread, sel};
use objc2_foundation::{NSArray, NSData, NSDictionary};
use objc2_vision::{VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};

use super::Status;
use crate::{PlatformError, Result};

pub fn status() -> Status {
    Status { engine: "Apple Vision", available: true, detail: "Built into macOS; the language is detected automatically.".into() }
}

pub fn recognize(png: &[u8]) -> Result<String> {
    autoreleasepool(|_| {
        let data = NSData::with_bytes(png);
        let options = NSDictionary::<VNImageOption, AnyObject>::new();
        let handler = VNImageRequestHandler::initWithData_options(VNImageRequestHandler::alloc(), &data, &options);

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        // macOS 13+; before that Vision reads English unless told otherwise.
        if request.respondsToSelector(sel!(setAutomaticallyDetectsLanguage:)) {
            request.setAutomaticallyDetectsLanguage(true);
        }

        let as_request: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[as_request]))
            .map_err(|e| PlatformError::Ocr(e.localizedDescription().to_string()))?;

        // One observation per line of text, top to bottom.
        let mut lines = Vec::new();
        for observation in request.results().iter().flat_map(|results| results.iter()) {
            if let Some(best) = observation.topCandidates(1).firstObject() {
                lines.push(best.string().to_string());
            }
        }
        Ok(lines.join("\n"))
    })
}
