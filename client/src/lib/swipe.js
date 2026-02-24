export function swipeNav(node, opts) {
	let startX, startY, locked, horizontal;
	const { onLeft, onRight } = opts;

	function onStart(e) {
		const touch = e.touches[0];
		startX = touch.clientX;
		startY = touch.clientY;
		locked = false;
		horizontal = false;
	}

	function onMove(e) {
		const touch = e.touches[0];
		const dx = touch.clientX - startX;
		const dy = touch.clientY - startY;

		if (!locked) {
			if (Math.abs(dx) < 5 && Math.abs(dy) < 5) return;
			locked = true;
			horizontal = Math.abs(dx) > Math.abs(dy);
		}
		if (!horizontal) return;

		e.preventDefault();
	}

	function onEnd(e) {
		if (!locked || !horizontal) return;
		const dx = e.changedTouches[0].clientX - startX;
		if (dx < -50 && onLeft) onLeft();
		if (dx > 50 && onRight) onRight();
	}

	node.addEventListener('touchstart', onStart, { passive: true });
	node.addEventListener('touchmove', onMove, { passive: false });
	node.addEventListener('touchend', onEnd, { passive: true });

	return {
		destroy() {
			node.removeEventListener('touchstart', onStart);
			node.removeEventListener('touchmove', onMove);
			node.removeEventListener('touchend', onEnd);
		},
	};
}

export function swipeable(node, opts) {
	let startX, startY, offsetX, locked, horizontal;
	let swipeBg;
	const { onSwipeLeft, onSwipeRight } = opts;

	function preventClick(e) {
		e.stopPropagation();
		e.preventDefault();
	}

	function onStart(e) {
		const touch = e.touches[0];
		startX = touch.clientX;
		startY = touch.clientY;
		offsetX = 0;
		locked = false;
		horizontal = false;
		swipeBg = node.previousElementSibling;
		node.style.transition = 'none';
		if (swipeBg) swipeBg.style.transition = 'none';
	}

	function onMove(e) {
		const touch = e.touches[0];
		const dx = touch.clientX - startX;
		const dy = touch.clientY - startY;

		if (!locked) {
			if (Math.abs(dx) < 5 && Math.abs(dy) < 5) return;
			locked = true;
			horizontal = Math.abs(dx) > Math.abs(dy);
		}
		if (!horizontal) return;

		e.preventDefault();
		const maxLeft = onSwipeLeft ? -80 : 0;
		const maxRight = onSwipeRight ? 80 : 0;
		offsetX = Math.max(maxLeft, Math.min(maxRight, dx));
		node.style.transform = `translateX(${offsetX}px)`;
		if (swipeBg) swipeBg.style.opacity = Math.min(1, Math.abs(offsetX) / 40);
	}

	function onEnd() {
		if (!locked) return;
		if (horizontal) {
			node.addEventListener('click', preventClick, { once: true, capture: true });
			if (offsetX < -40 && onSwipeLeft) onSwipeLeft();
			if (offsetX > 40 && onSwipeRight) onSwipeRight();
		}
		node.style.transition = 'transform 0.2s ease';
		node.style.transform = 'translateX(0)';
		if (swipeBg) {
			swipeBg.style.transition = 'opacity 0.2s ease';
			swipeBg.style.opacity = 0;
		}
		offsetX = 0;
	}

	node.addEventListener('touchstart', onStart, { passive: true });
	node.addEventListener('touchmove', onMove, { passive: false });
	node.addEventListener('touchend', onEnd, { passive: true });

	return {
		destroy() {
			node.removeEventListener('touchstart', onStart);
			node.removeEventListener('touchmove', onMove);
			node.removeEventListener('touchend', onEnd);
		},
	};
}
